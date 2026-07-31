use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::json;

use crate::flows::{FlowError, FlowResult};
use crate::herdr::HerdrClient;
use crate::registry;
use crate::registry::ssh::SshSource;

const WAIT_ATTEMPTS: usize = 5;
const WAIT_INTERVAL: Duration = Duration::from_millis(100);

static CAUGHT_SIGNAL: AtomicU32 = AtomicU32::new(0);
static SIGNAL_PIPE_WRITE: AtomicI32 = AtomicI32::new(-1);

/// The four fields of one `Host` block, exactly as the prompts collect them.
///
/// The port stays a `String` because the zsh original validated the *text* the user
/// typed, not a parsed number: `+22` and ` 22` were refused before any arithmetic ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshEditFields {
    pub alias: String,
    pub hostname: String,
    pub user: String,
    pub port: String,
}

/// Validate the four fields as one pure step, so the tests never drive a prompt.
pub fn validate_edit_fields(fields: &SshEditFields) -> std::result::Result<(), String> {
    if !valid_name(&fields.alias) {
        return Err(format!("Invalid SSH alias: {}", fields.alias));
    }
    if fields.hostname.is_empty() || fields.hostname.chars().any(char::is_whitespace) {
        return Err(format!("Invalid HostName: {}", fields.hostname));
    }
    if !fields.user.is_empty() && !valid_name(&fields.user) {
        return Err(format!("Invalid SSH user: {}", fields.user));
    }
    if !valid_port(&fields.port) {
        return Err(format!("Invalid SSH port: {}", fields.port));
    }
    Ok(())
}

/// Refuse an alias the config already declares.
///
/// `ssh` resolves the *first* matching `Host` block, so a second block with the same
/// alias is silently dead config. The keyword is matched case-insensitively and the
/// alias exactly, which is what the zsh `awk` guard did.
pub fn validate_alias_available(contents: &str, alias: &str) -> std::result::Result<(), String> {
    if contains_alias(contents, alias) {
        return Err(format!("SSH alias already exists: {alias}"));
    }
    Ok(())
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
}

// zsh ran `grep -qxE '[0-9]+'` *before* the range test, so anything that is not a run
// of digits was rejected without being parsed. `u16::from_str` would accept `+22`.
fn valid_port(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value
            .parse::<u32>()
            .is_ok_and(|port| (1..=65535).contains(&port))
}

/// Collect one SSH host block from prompts, or open the config for an existing one.
pub fn edit(
    target: Option<&str>,
    config: &Path,
    registry_file: &Path,
    history_file: &Path,
) -> Result<()> {
    edit_with(target, config, registry_file, history_file, &mut GumPrompt)
}

fn edit_with(
    target: Option<&str>,
    config: &Path,
    registry_file: &Path,
    history_file: &Path,
    prompt: &mut dyn EditPrompt,
) -> Result<()> {
    // The picker binds `ctrl-i:execute(<self> ssh edit {2})`, and fzf expands `{2}` to
    // an empty string when no row is selected. Bail before any I/O, and before any
    // prompt: falling through would offer to add a host with blank defaults.
    let target = target
        .filter(|target| !target.is_empty())
        .context("No SSH target selected.")?;

    let registry_data =
        registry::ssh::sync(registry_file, config, history_file).context("ssh edit")?;
    let entry = registry_data
        .targets
        .get(target)
        .with_context(|| format!("SSH target not found: {target}"))
        .context("ssh edit")?;

    // A host that came from the config file already has a block; editing its fields
    // belongs to the user's editor, not to this program.
    if entry.source == SshSource::Config {
        return exec_editor(config);
    }

    let config_contents = match fs::read_to_string(config) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).context("ssh edit: read SSH config"),
    };

    clear_screen()?;

    let (default_hostname, default_user) = target_defaults(target);
    let Some(alias) = prompt.input("Create SSH config", "Alias › ", "", "server-name")? else {
        return Ok(());
    };
    let Some(hostname) = prompt.input("", "HostName › ", &default_hostname, "")? else {
        return Ok(());
    };
    let Some(user) = prompt.input("", "User › ", &default_user, "")? else {
        return Ok(());
    };
    let Some(port) = prompt.input("", "Port › ", "22", "")? else {
        return Ok(());
    };
    let fields = SshEditFields {
        alias,
        hostname,
        user,
        port,
    };
    // Show the exact block that will be appended, framed by blank lines, then confirm.
    print!("\n{}\n", render_block(&fields, false));
    io::stdout().flush().context("ssh edit: flush preview")?;
    if !prompt.confirm("Add this host to SSH config?")? {
        return Ok(());
    }

    validate_edit_fields(&fields).map_err(anyhow::Error::msg)?;
    validate_alias_available(&config_contents, &fields.alias).map_err(anyhow::Error::msg)?;
    write_config_atomically(config, &config_contents, &fields)?;
    registry::ssh::sync(registry_file, config, history_file).context("ssh edit")?;
    registry::ssh::use_target(registry_file, config, history_file, target).context("ssh edit")?;
    println!("Added SSH config: {}", fields.alias);
    Ok(())
}

/// Every question the editor asks, so the tests can answer them without a terminal.
///
/// `None` from `input`, or `false` from `confirm`, is a cancellation: the flow returns
/// success and writes nothing, exactly as the zsh `|| exit 0` chain did.
trait EditPrompt {
    fn input(
        &mut self,
        header: &str,
        prompt: &str,
        value: &str,
        placeholder: &str,
    ) -> Result<Option<String>>;

    fn confirm(&mut self, prompt: &str) -> Result<bool>;
}

struct GumPrompt;

impl EditPrompt for GumPrompt {
    fn input(
        &mut self,
        header: &str,
        prompt: &str,
        value: &str,
        placeholder: &str,
    ) -> Result<Option<String>> {
        gum_input(header, prompt, value, placeholder)
    }

    fn confirm(&mut self, prompt: &str) -> Result<bool> {
        gum_confirm(prompt)
    }
}

/// Pre-fill the prompts from the target itself: `user@hostname` fills both fields,
/// anything else is a bare hostname. The alias gets a placeholder instead of a value,
/// because it names the shortcut the user wants to type, not the host they already have.
fn target_defaults(target: &str) -> (String, String) {
    // zsh split on the first `@` only (`${target%%@*}` / `${target#*@}`).
    match target.split_once('@') {
        Some((user, hostname)) => (hostname.to_owned(), user.to_owned()),
        None => (target.to_owned(), String::new()),
    }
}

/// Hand the editor a clean screen.
///
/// This runs from an fzf `execute` binding, and fzf owns the alternate screen: it has
/// already drawn the picker there and will redraw it when this command exits. Drawing
/// prompts over that leaves the picker's rows interleaved with them, and fzf's redraw
/// then only repaints the region it believes it owns, so the leftovers stay on screen.
/// Clearing first gives the prompts the full pane and fzf a clean surface to restore.
fn clear_screen() -> Result<()> {
    Command::new("clear")
        .status()
        .context("failed to clear screen")?;
    Ok(())
}

fn contains_alias(contents: &str, wanted: &str) -> bool {
    contents.lines().any(|line| {
        let mut fields = line.split_whitespace();
        fields
            .next()
            .is_some_and(|field| field.eq_ignore_ascii_case("host"))
            && fields.any(|alias| alias == wanted)
    })
}

fn resolve_editor(visual: Option<OsString>, editor: Option<OsString>) -> OsString {
    visual
        .filter(|value| !value.is_empty())
        .or_else(|| editor.filter(|value| !value.is_empty()))
        .unwrap_or_else(|| OsString::from("nvim"))
}

// zsh `exec`d the editor, and that is worth keeping: fzf's `execute` binding waits on
// this pid, so replacing the process hands the editor the terminal for its whole run
// and leaves no wrapper to outlive it or to reinterpret its exit status.
fn exec_editor(config: &Path) -> Result<()> {
    let editor = resolve_editor(std::env::var_os("VISUAL"), std::env::var_os("EDITOR"));
    let error = Command::new(&editor).arg(config).exec();
    Err(error).with_context(|| {
        format!(
            "ssh edit: failed to open editor {}",
            editor.to_string_lossy()
        )
    })
}

fn gum_input(header: &str, prompt: &str, value: &str, placeholder: &str) -> Result<Option<String>> {
    let mut command = Command::new("gum");
    command.arg("input");
    if !header.is_empty() {
        command.arg(format!("--header={header}"));
    }
    if !placeholder.is_empty() {
        command.arg(format!("--placeholder={placeholder}"));
    }
    // gum draws its UI on the controlling TTY and writes the answer to stdout, so
    // capturing stdout still shows the prompt. A non-zero exit means cancelled.
    let output = command
        .arg(format!("--prompt={prompt}"))
        .arg(format!("--value={value}"))
        .output()
        .context("failed to run gum input")?;
    Ok(output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .trim_end_matches(['\r', '\n'])
            .to_owned()
    }))
}

fn gum_confirm(prompt: &str) -> Result<bool> {
    Ok(Command::new("gum")
        .args(["confirm", prompt])
        .status()
        .context("failed to run gum confirm")?
        .success())
}

/// Render one `Host` block. `User` is omitted entirely when no user was given, and the
/// leading blank line separates the block from a non-empty file.
fn render_block(fields: &SshEditFields, leading_blank_line: bool) -> String {
    let mut block = String::new();
    if leading_blank_line {
        block.push('\n');
    }
    block.push_str(&format!(
        "Host {}\n  HostName {}\n",
        fields.alias, fields.hostname
    ));
    if !fields.user.is_empty() {
        block.push_str(&format!("  User {}\n", fields.user));
    }
    block.push_str(&format!("  Port {}\n", fields.port));
    block
}

/// Append the block by replacing the whole file, never by writing it in place.
///
/// This is the reason the copy-append-rename dance exists: `~/.ssh/config` is the only
/// record of how the user reaches every one of their machines, and an in-place append
/// that dies part-way through — a full disk, a killed popup, a `^C` — leaves a
/// truncated `Host` line that `ssh` refuses to parse. The whole file is then unusable
/// and every host is unreachable, to fix a file the user never asked to have touched.
/// `rename(2)` within one directory is atomic, so a reader sees either the old config
/// or the new one, and a failure anywhere before the rename leaves the original intact.
///
/// The copy preserves the mode: SSH ignores a config that is group- or world-writable.
fn write_config_atomically(config: &Path, existing: &str, fields: &SshEditFields) -> Result<()> {
    write_config_atomically_with(config, existing, fields, |source, destination| {
        fs::rename(source, destination)
    })
}

/// The same write, with `replace` injected so a test can fail the rename and check that
/// the original config survived and no temporary file was left behind.
fn write_config_atomically_with<F>(
    config: &Path,
    existing: &str,
    fields: &SshEditFields,
    replace: F,
) -> Result<()>
where
    F: FnOnce(&Path, &Path) -> io::Result<()>,
{
    let parent = config
        .parent()
        .with_context(|| format!("SSH config has no parent: {}", config.display()))?;
    fs::create_dir_all(parent).context("create SSH config directory")?;
    let file_name = config
        .file_name()
        .with_context(|| format!("SSH config has no file name: {}", config.display()))?;
    let temp = parent.join(format!(
        ".{}.tmp-{}",
        file_name.to_string_lossy(),
        std::process::id()
    ));

    let result = (|| -> Result<()> {
        if config.exists() {
            fs::copy(config, &temp).context("copy SSH config to temporary file")?;
            let mode = fs::metadata(config)?.permissions().mode();
            fs::set_permissions(&temp, fs::Permissions::from_mode(mode))?;
        } else {
            fs::write(&temp, []).context("create temporary SSH config")?;
        }
        let mut file = OpenOptions::new().append(true).open(&temp)?;
        if !existing.is_empty() && !existing.ends_with('\n') {
            file.write_all(b"\n")?;
        }
        file.write_all(render_block(fields, !existing.is_empty()).as_bytes())?;
        file.sync_all()?;
        drop(file);
        replace(&temp, config).context("replace SSH config")?;
        Ok(())
    })();

    if result.is_err() && temp.exists() {
        let _ = fs::remove_file(&temp);
    }
    result
}

extern "C" fn signal_handler(signal: libc::c_int) {
    CAUGHT_SIGNAL.store(signal as u32, Ordering::Relaxed);
    let fd = SIGNAL_PIPE_WRITE.load(Ordering::Relaxed);
    if fd >= 0 {
        let byte = signal as u8;
        // SAFETY: write is async-signal-safe, and both pointers and length are valid.
        unsafe {
            libc::write(fd, (&byte as *const u8).cast(), 1);
        }
    }
}

struct SignalHandlers {
    read_fd: libc::c_int,
    write_fd: libc::c_int,
    previous: [libc::sigaction; 3],
}

impl SignalHandlers {
    fn install() -> io::Result<Self> {
        let mut pipe = [-1; 2];
        // SAFETY: pipe points to storage for two file descriptors.
        if unsafe { libc::pipe(pipe.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        // A full pipe must never make the signal handler block.
        unsafe {
            libc::fcntl(pipe[1], libc::F_SETFL, libc::O_NONBLOCK);
        }

        CAUGHT_SIGNAL.store(0, Ordering::Relaxed);
        SIGNAL_PIPE_WRITE.store(pipe[1], Ordering::Relaxed);
        let signals = [libc::SIGHUP, libc::SIGINT, libc::SIGTERM];
        let mut previous = [
            unsafe { std::mem::zeroed() },
            unsafe { std::mem::zeroed() },
            unsafe { std::mem::zeroed() },
        ];

        for (index, signal) in signals.into_iter().enumerate() {
            // SAFETY: sigaction is initialized before use, and its mask is valid.
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = signal_handler as *const () as usize;
            action.sa_flags = 0;
            unsafe {
                libc::sigemptyset(&mut action.sa_mask);
            }
            // SAFETY: pointers refer to initialized, correctly sized sigaction values.
            if unsafe { libc::sigaction(signal, &action, &mut previous[index]) } == -1 {
                for restore_index in 0..index {
                    unsafe {
                        libc::sigaction(
                            signals[restore_index],
                            &previous[restore_index],
                            std::ptr::null_mut(),
                        );
                    }
                }
                SIGNAL_PIPE_WRITE.store(-1, Ordering::Relaxed);
                unsafe {
                    libc::close(pipe[0]);
                    libc::close(pipe[1]);
                }
                return Err(io::Error::last_os_error());
            }
        }

        Ok(Self {
            read_fd: pipe[0],
            write_fd: pipe[1],
            previous,
        })
    }
}

impl Drop for SignalHandlers {
    fn drop(&mut self) {
        SIGNAL_PIPE_WRITE.store(-1, Ordering::Relaxed);
        for (index, signal) in [libc::SIGHUP, libc::SIGINT, libc::SIGTERM]
            .into_iter()
            .enumerate()
        {
            // SAFETY: previous contains the handlers returned by sigaction.
            unsafe {
                libc::sigaction(signal, &self.previous[index], std::ptr::null_mut());
            }
        }
        // SAFETY: these descriptors are owned by this guard.
        unsafe {
            libc::close(self.read_fd);
            libc::close(self.write_fd);
        }
    }
}

/// Own one SSH connection, close its dedicated tab, then exit with ssh's own status.
///
/// On success this never returns: it ends the process with the status the connection
/// ended on, mirroring `exit "$ssh_status"` in the zsh original, and `128 + signum`
/// when a signal ended it instead. The status is the only thing this wrapper adds over
/// bare `ssh`, so discarding it and returning `Outcome::Done` would make every session
/// look successful. It still returns `FlowResult` so a failure *before* that point —
/// the signal handlers, or the spawn — routes through the single reporting path in
/// `main` and reaches the user as a notification.
pub fn session(
    target: &str,
    tab_id: &str,
    registry_file: &Path,
    history_file: &Path,
    client: &dyn HerdrClient,
) -> FlowResult {
    let mut command = Command::new("ssh");
    command.arg(target);
    let exit_code = session_with_command(
        target,
        tab_id,
        registry_file,
        history_file,
        client,
        &mut command,
    )?;
    // Nothing is buffered at this point: the child owned the terminal directly and the
    // tab is already closed, so there is nothing left for a destructor to flush.
    std::process::exit(exit_code)
}

/// The same session with the child command injected, returning the status instead of
/// exiting so a test can assert both the status and the reporting metadata.
fn session_with_command(
    target: &str,
    tab_id: &str,
    registry_file: &Path,
    history_file: &Path,
    client: &dyn HerdrClient,
    command: &mut Command,
) -> Result<i32> {
    run_session(target, tab_id, registry_file, history_file, client, command)
        .map_err(|error| anyhow::Error::from(FlowError::titled("SSH session", error)))
}

fn run_session(
    target: &str,
    tab_id: &str,
    registry_file: &Path,
    history_file: &Path,
    client: &dyn HerdrClient,
    command: &mut Command,
) -> Result<i32> {
    let result = (|| {
        let handlers = SignalHandlers::install().context("ssh session: install signal handlers")?;

        // Agent launch uses exec because the harness replaces the launcher. SSH must
        // remain a child because this process closes the dedicated tab after it exits.
        let child = command
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .context("ssh session: spawning ssh")?;
        wait_for_child(child, handlers.read_fd)
    })();

    // The tab exists only to hold this connection, so it must close whether ssh exited,
    // was signalled, or never started — a tab left open after a failed connection is a
    // worse outcome than a missing notification. A close failure is deliberately
    // swallowed so it cannot displace the real cause on its way to `main`.
    let _ = client.tab_close(json!({"id": tab_id}));
    let (status, caught_signal) = result?;

    let exit_code = if let Some(signal) = caught_signal {
        128 + signal
    } else {
        status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(libc::SIGKILL))
    };

    if exit_code == 0 {
        // Bookkeeping, and best-effort by design: the tab is already gone, so there is
        // no pane left to print to and no notification worth raising for a stamp the
        // user never asked for. Neither failure may change the status ssh exited with.
        let _ =
            registry::ssh::use_target(registry_file, Path::new("/dev/null"), history_file, target);
        let _ = append_history(history_file, target, unix_epoch()?);
    }

    Ok(exit_code)
}

fn wait_for_child(
    mut child: Child,
    signal_pipe: libc::c_int,
) -> Result<(std::process::ExitStatus, Option<i32>)> {
    loop {
        match wait_pid(&mut child, libc::WNOHANG) {
            Ok(Some(status)) => return Ok((status, None)),
            Ok(None) => {
                let signal = CAUGHT_SIGNAL.swap(0, Ordering::Relaxed) as i32;
                if signal != 0 {
                    return terminate_child(child, signal);
                }
                wait_for_signal_pipe(signal_pipe)?;
            }
            Err(error) if error.raw_os_error() == Some(libc::EINTR) => {
                let signal = CAUGHT_SIGNAL.swap(0, Ordering::Relaxed) as i32;
                if signal == 0 {
                    continue;
                }
                return terminate_child(child, signal);
            }
            Err(error) => return Err(error).context("failed to wait for SSH child"),
        }
    }
}

fn wait_for_signal_pipe(fd: libc::c_int) -> io::Result<()> {
    let mut descriptor = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // A timeout also observes a child that exits without writing to the pipe.
    let result = unsafe { libc::poll(&mut descriptor, 1, 50) };
    if result == -1 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(error);
        }
    } else if result > 0 {
        let mut byte = 0_u8;
        unsafe {
            libc::read(fd, (&mut byte as *mut u8).cast(), 1);
        }
    }
    Ok(())
}

fn terminate_child(
    mut child: Child,
    signal: i32,
) -> Result<(std::process::ExitStatus, Option<i32>)> {
    // SAFETY: the PID belongs to the live child and signal was caught above.
    unsafe {
        libc::kill(child.id() as libc::pid_t, signal);
    }
    for _ in 0..WAIT_ATTEMPTS {
        if let Some(status) = wait_pid(&mut child, libc::WNOHANG)? {
            return Ok((status, Some(signal)));
        }
        std::thread::sleep(WAIT_INTERVAL);
    }
    // SAFETY: the child is still live after the bounded wait.
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGKILL);
    }
    let status = loop {
        match wait_pid(&mut child, 0) {
            Ok(Some(status)) => break status,
            Err(error) if error.raw_os_error() == Some(libc::EINTR) => continue,
            Ok(None) => unreachable!("blocking waitpid returned no status"),
            Err(error) => return Err(error).context("failed to reap SSH child"),
        }
    };
    Ok((status, Some(signal)))
}

fn wait_pid(
    child: &mut Child,
    options: libc::c_int,
) -> io::Result<Option<std::process::ExitStatus>> {
    let mut raw_status = 0;
    // SAFETY: raw_status is writable and child.id() identifies this process's child.
    let result = unsafe { libc::waitpid(child.id() as libc::pid_t, &mut raw_status, options) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else if result == 0 {
        Ok(None)
    } else {
        Ok(Some(std::process::ExitStatus::from_raw(raw_status)))
    }
}

fn unix_epoch() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_secs())
}

fn history_line(target: &str, epoch: u64) -> String {
    format!(": {epoch}:0;ssh {target}\n")
}

fn append_history(path: &Path, target: &str, epoch: u64) -> io::Result<()> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(history_line(target, epoch).as_bytes())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::sync::Mutex;

    use crate::herdr::FakeClient;

    use super::*;

    static SIGNAL_TEST: Mutex<()> = Mutex::new(());

    fn valid_fields(user: &str) -> SshEditFields {
        SshEditFields {
            alias: "myhost".to_owned(),
            hostname: "example.com".to_owned(),
            user: user.to_owned(),
            port: "2222".to_owned(),
        }
    }

    #[test]
    fn missing_and_empty_targets_fail_before_ui() {
        // Every path points at a file that does not exist, and the prompt panics if it
        // is asked anything: reaching either would mean the guard came too late.
        let path = temp_path("unused-config");
        let _ = fs::remove_file(&path);
        for target in [None, Some("")] {
            let mut prompt = ScriptedPrompt::default();
            let error = edit_with(target, &path, &path, &path, &mut prompt).unwrap_err();

            assert_eq!(error.to_string(), "No SSH target selected.");
            assert!(prompt.inputs.is_empty());
            assert!(!prompt.confirmed);
            assert!(!path.exists());
        }
    }

    #[test]
    fn derives_manual_target_defaults() {
        assert_eq!(
            target_defaults("alice@example.com"),
            ("example.com".to_owned(), "alice".to_owned())
        );
        assert_eq!(
            target_defaults("example.com"),
            ("example.com".to_owned(), String::new())
        );
        // zsh split on the first `@`, so the rest stays in the hostname.
        assert_eq!(target_defaults("a@b@c"), ("b@c".to_owned(), "a".to_owned()));
    }

    #[test]
    fn prefers_visual_then_editor_then_nvim() {
        let visual = OsString::from("visual-editor");
        let editor = OsString::from("editor");
        let cases = [
            (Some(visual.clone()), Some(editor.clone()), "visual-editor"),
            (None, Some(editor.clone()), "editor"),
            (Some(OsString::new()), Some(editor.clone()), "editor"),
            (Some(visual.clone()), None, "visual-editor"),
            (None, None, "nvim"),
            (Some(OsString::new()), Some(OsString::new()), "nvim"),
            (None, Some(OsString::new()), "nvim"),
        ];

        for (visual, editor, expected) in cases {
            assert_eq!(
                resolve_editor(visual.clone(), editor.clone()),
                OsString::from(expected),
                "VISUAL={visual:?} EDITOR={editor:?}"
            );
        }
    }

    #[test]
    fn validates_every_ssh_config_field() {
        let valid = valid_fields("alice");
        assert_eq!(validate_edit_fields(&valid), Ok(()));

        let cases = [
            ("alias", "bad alias", "Invalid SSH alias: bad alias"),
            ("hostname", "", "Invalid HostName: "),
            ("hostname", "bad hostname", "Invalid HostName: bad hostname"),
            ("user", "bad user", "Invalid SSH user: bad user"),
            ("port", "0", "Invalid SSH port: 0"),
            ("port", "65536", "Invalid SSH port: 65536"),
            ("port", "abc", "Invalid SSH port: abc"),
            ("port", "", "Invalid SSH port: "),
            // `grep -qxE '[0-9]+'` refused a sign, and so must the port parser.
            ("port", "+22", "Invalid SSH port: +22"),
            ("port", " 22", "Invalid SSH port:  22"),
        ];
        for (field, value, expected) in cases {
            let mut fields = valid.clone();
            match field {
                "alias" => fields.alias = value.to_owned(),
                "hostname" => fields.hostname = value.to_owned(),
                "user" => fields.user = value.to_owned(),
                "port" => fields.port = value.to_owned(),
                _ => unreachable!(),
            }
            assert_eq!(validate_edit_fields(&fields), Err(expected.to_owned()));
        }
        assert_eq!(
            validate_alias_available("hOsT other myhost\n", "myhost"),
            Err("SSH alias already exists: myhost".to_owned())
        );
        assert_eq!(validate_alias_available("Host MyHost\n", "myhost"), Ok(()));
    }

    /// Answers the four inputs and the confirm from a script, and records what it was
    /// asked, so the prompt flow can be tested without gum or a terminal.
    #[derive(Default)]
    struct ScriptedPrompt {
        answers: std::collections::VecDeque<Option<String>>,
        confirm: Option<bool>,
        inputs: Vec<[String; 4]>,
        confirmed: bool,
    }

    impl EditPrompt for ScriptedPrompt {
        fn input(
            &mut self,
            header: &str,
            prompt: &str,
            value: &str,
            placeholder: &str,
        ) -> Result<Option<String>> {
            self.inputs.push([
                header.to_owned(),
                prompt.to_owned(),
                value.to_owned(),
                placeholder.to_owned(),
            ]);
            Ok(self.answers.pop_front().expect("unscripted prompt"))
        }

        fn confirm(&mut self, _prompt: &str) -> Result<bool> {
            self.confirmed = true;
            Ok(self.confirm.expect("unscripted confirm"))
        }
    }

    struct Sandbox {
        dir: std::path::PathBuf,
        config: std::path::PathBuf,
        registry: std::path::PathBuf,
        history: std::path::PathBuf,
    }

    impl Sandbox {
        /// An empty SSH config keeps `sync` from shelling out to `ssh -G`, and the
        /// registry entry is `manual` so the flow takes the prompt path.
        fn new(name: &str, config_contents: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("workbench-ssh-edit-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            let sandbox = Self {
                config: dir.join("config"),
                registry: dir.join("registry.json"),
                history: dir.join("history"),
                dir,
            };
            fs::write(&sandbox.config, config_contents).unwrap();
            fs::write(
                &sandbox.registry,
                r#"{"version":1,"targets":{"alice@example.com":{"source":"manual",
                   "hostname":null,"user":null,"aliases":null,
                   "last_used_at":null,"hidden":false}}}"#,
            )
            .unwrap();
            fs::write(&sandbox.history, "").unwrap();
            sandbox
        }

        fn edit(&self, prompt: &mut ScriptedPrompt) -> Result<()> {
            edit_with(
                Some("alice@example.com"),
                &self.config,
                &self.registry,
                &self.history,
                prompt,
            )
        }

        fn file_names(&self) -> Vec<String> {
            let mut names = fs::read_dir(&self.dir)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            names.sort();
            names
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn cancelling_any_prompt_writes_nothing_and_exits_zero() {
        let answers = ["myhost", "example.com", "alice", "2222"];

        for cancel_at in 0..=answers.len() {
            let sandbox = Sandbox::new(&format!("cancel-{cancel_at}"), "");
            let mut prompt = ScriptedPrompt::default();
            for answer in answers.iter().take(cancel_at) {
                prompt.answers.push_back(Some((*answer).to_owned()));
            }
            // The last case answers every input and then declines the confirm.
            if cancel_at < answers.len() {
                prompt.answers.push_back(None);
            } else {
                prompt.confirm = Some(false);
            }

            sandbox.edit(&mut prompt).unwrap();

            assert_eq!(fs::read_to_string(&sandbox.config).unwrap(), "");
            assert_eq!(prompt.confirmed, cancel_at == answers.len());
            assert_eq!(
                prompt.inputs.len(),
                (cancel_at + 1).min(answers.len()),
                "cancelling at {cancel_at} asked the wrong number of questions"
            );
            assert_eq!(
                sandbox.file_names(),
                ["config", "history", "registry.json"],
                "cancelling at {cancel_at} left a file behind"
            );
        }
    }

    #[test]
    fn prompts_carry_the_documented_defaults() {
        let sandbox = Sandbox::new("defaults", "");
        let mut prompt = ScriptedPrompt::default();
        for _ in 0..3 {
            prompt.answers.push_back(Some("x".to_owned()));
        }
        prompt.answers.push_back(None);

        sandbox.edit(&mut prompt).unwrap();

        assert_eq!(
            prompt.inputs,
            [
                ["Create SSH config", "Alias › ", "", "server-name"],
                ["", "HostName › ", "example.com", ""],
                ["", "User › ", "alice", ""],
                ["", "Port › ", "22", ""],
            ]
        );
    }

    #[test]
    fn a_refused_alias_after_the_confirm_still_writes_nothing() {
        let existing = "Host myhost\n  HostName old.example.com\n";
        let sandbox = Sandbox::new("duplicate", existing);
        let mut prompt = ScriptedPrompt::default();
        for answer in ["myhost", "example.com", "alice", "2222"] {
            prompt.answers.push_back(Some(answer.to_owned()));
        }
        prompt.confirm = Some(true);

        let error = sandbox.edit(&mut prompt).unwrap_err();

        assert_eq!(error.to_string(), "SSH alias already exists: myhost");
        assert_eq!(fs::read_to_string(&sandbox.config).unwrap(), existing);
        assert_eq!(sandbox.file_names(), ["config", "history", "registry.json"]);
    }

    #[test]
    fn renders_exact_block_bytes_for_all_file_and_user_combinations() {
        let cases = [
            (
                "",
                "alice",
                "Host myhost\n  HostName example.com\n  User alice\n  Port 2222\n",
            ),
            ("", "", "Host myhost\n  HostName example.com\n  Port 2222\n"),
            (
                "Host old\n",
                "alice",
                "Host old\n\nHost myhost\n  HostName example.com\n  User alice\n  Port 2222\n",
            ),
            (
                "Host old",
                "",
                "Host old\n\nHost myhost\n  HostName example.com\n  Port 2222\n",
            ),
        ];

        for (index, (existing, user, expected)) in cases.into_iter().enumerate() {
            let path = temp_path(&format!("block-{index}"));
            let _ = fs::remove_file(&path);
            if !existing.is_empty() {
                fs::write(&path, existing).unwrap();
            }
            write_config_atomically(&path, existing, &valid_fields(user)).unwrap();
            assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn failed_replace_preserves_original_and_removes_temporary_file() {
        let path = temp_path("replace-failure");
        let temporary = path.with_file_name(format!(
            ".{}.tmp-{}",
            path.file_name().unwrap().to_string_lossy(),
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&temporary);
        fs::write(&path, "Host old\n").unwrap();

        let result =
            write_config_atomically_with(&path, "Host old\n", &valid_fields("alice"), |_, _| {
                Err(io::Error::other("simulated replace failure"))
            });

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "Host old\n");
        assert!(!temporary.exists());
        fs::remove_file(path).unwrap();
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("workbench-ssh-{}-{name}", std::process::id()))
    }

    fn run_shell(script: &str, client: &FakeClient) -> (i32, std::path::PathBuf) {
        let registry = temp_path("registry.json");
        let history = temp_path("history");
        let _ = fs::remove_file(&registry);
        let _ = fs::remove_file(&history);
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]);
        // `session_with_command` is the same flow `session` runs, minus the final
        // `std::process::exit` — so the status these tests assert is the status the
        // process leaves with.
        let code = session_with_command(
            "test-host",
            "tab-id",
            &registry,
            &history,
            client,
            &mut command,
        )
        .unwrap();
        (code, registry)
    }

    #[test]
    fn formats_zsh_extended_history_line() {
        assert_eq!(
            history_line("user@example.com", 1_722_222_222),
            ": 1722222222:0;ssh user@example.com\n"
        );
    }

    #[test]
    fn zero_exit_closes_tab_and_stamps_registry() {
        let _guard = SIGNAL_TEST.lock().unwrap();
        let client = FakeClient::default();
        let (code, registry) = run_shell("exit 0", &client);

        assert_eq!(code, 0);
        assert_eq!(
            client.calls.borrow().as_slice(),
            &[("tab.close".to_owned(), json!({"id": "tab-id"}))]
        );
        assert!(fs::read_to_string(registry)
            .unwrap()
            .contains("\"last_used_at\":"));
    }

    #[test]
    fn nonzero_exit_closes_tab_without_stamping_registry() {
        let _guard = SIGNAL_TEST.lock().unwrap();
        let client = FakeClient::default();
        let (code, registry) = run_shell("exit 23", &client);

        assert_eq!(code, 23);
        assert_eq!(client.calls.borrow()[0].0, "tab.close");
        assert!(!registry.exists());
    }

    #[test]
    fn spawn_failure_closes_tab_and_has_exact_reporting_metadata() {
        let client = FakeClient::default();
        let mut command = Command::new("/definitely/missing/ssh");

        let error = session_with_command(
            "test-host",
            "tab-id",
            Path::new("/unused/registry"),
            Path::new("/unused/history"),
            &client,
            &mut command,
        )
        .expect_err("reject spawn failure");
        let flow_error = error.downcast_ref::<FlowError>().unwrap();

        assert_eq!(flow_error.title(), Some("SSH session"));
        assert_eq!(flow_error.prefix(), None);
        assert!(flow_error.chain().starts_with("ssh session: spawning ssh:"));
        assert_eq!(
            client.calls.into_inner(),
            vec![("tab.close".to_owned(), json!({"id": "tab-id"}))]
        );
    }

    fn assert_signal(signal: i32, script: &str) {
        let _guard = SIGNAL_TEST.lock().unwrap();
        let client = FakeClient::default();
        let sender = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            unsafe {
                libc::kill(std::process::id() as libc::pid_t, signal);
            }
        });
        let (code, _) = run_shell(script, &client);
        sender.join().unwrap();

        assert_eq!(code, 128 + signal);
        assert_eq!(client.calls.borrow()[0].0, "tab.close");
    }

    #[test]
    fn forwards_hup_int_and_term_and_reaps_child() {
        for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGTERM] {
            assert_signal(signal, "sleep 30");
        }
    }

    #[test]
    fn kills_child_that_ignores_forwarded_signal_after_timeout() {
        assert_signal(libc::SIGTERM, "trap '' TERM; sleep 30");
    }
}
