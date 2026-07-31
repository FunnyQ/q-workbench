//! `gum` draws its UI on stderr whenever stdout is not a terminal, so any helper that
//! captures a `gum` answer has to inherit stderr or the prompt renders nowhere and the
//! user answers a blank screen. The rule was fixed once in the agent popup and then
//! broken again in two other helpers, so these tests run the real binary against a stub
//! `gum` and assert the drawing actually reaches this process's stderr.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const DRAWN: &str = "GUM-DREW-THIS";

fn temporary_directory() -> PathBuf {
    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    let directory = std::env::temp_dir().join(format!(
        "workbench-gum-streams-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before Unix epoch")
            .as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&directory).expect("create temporary directory");
    directory
}

/// A `gum` that behaves like the real one: the frame on stderr, the answer on stdout.
fn stub_gum(directory: &Path) -> PathBuf {
    let bin = directory.join("bin");
    fs::create_dir_all(&bin).expect("create stub bin");
    let gum = bin.join("gum");
    fs::write(
        &gum,
        format!("#!/bin/sh\nprintf '{DRAWN} %s\\n' \"$1\" >&2\nprintf 'stub-answer\\n'\n"),
    )
    .expect("write stub gum");
    fs::set_permissions(&gum, fs::Permissions::from_mode(0o755)).expect("chmod stub gum");
    bin
}

fn workbench(directory: &Path, args: &[&str], extra_config: &str) -> std::process::Output {
    let home = directory.join("home");
    fs::create_dir_all(&home).expect("create home");
    let config = directory.join("config.toml");
    fs::write(
        &config,
        format!(
            "ssh_registry_file = \"{d}/ssh.json\"\n\
             ssh_config_file = \"{d}/ssh_config\"\n\
             ssh_history_file = \"{d}/ssh_history\"\n\
             project_registry_file = \"{d}/projects.json\"\n{extra_config}",
            d = directory.display()
        ),
    )
    .expect("write config");

    let bin = stub_gum(directory);
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new(env!("CARGO_BIN_EXE_workbench"))
        .args(args)
        .env("PATH", path)
        .env("HOME", &home)
        .env("Q_WORKBENCH_LOCAL_CONFIG", &config)
        .output()
        .expect("run workbench")
}

#[test]
fn ssh_edit_lets_gum_draw_on_stderr() {
    let directory = temporary_directory();
    // A manual target has no config block, so `ssh edit` runs the gum prompts rather
    // than handing the file to $EDITOR.
    fs::write(
        directory.join("ssh.json"),
        r#"{"version":1,"targets":{"me@box":{"source":"manual","last_used_at":null,"hidden":false}}}"#,
    )
    .expect("write ssh registry");

    let output = workbench(&directory, &["ssh", "edit", "me@box"], "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    // `gum confirm` runs through `status()` and always draws, so the marker has to name
    // the subcommand: only `input` proves the capturing helper inherited stderr.
    assert!(
        stderr.contains(&format!("{DRAWN} input")),
        "the gum prompt never reached the screen; stderr was {stderr:?}"
    );
}

#[test]
fn project_edit_lets_gum_draw_on_stderr() {
    let directory = temporary_directory();
    fs::write(
        directory.join("projects.json"),
        r#"{"version":1,"generated_at":"2026-08-01T00:00:00Z","projects":{"/tmp/Alpha":{"name":"Alpha","sources":["manual"],"aliases":[],"hidden":false}}}"#,
    )
    .expect("write project registry");

    let output = workbench(&directory, &["project", "edit", "/tmp/Alpha"], "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!("{DRAWN} input")),
        "the gum prompt never reached the screen; stderr was {stderr:?}"
    );
}
