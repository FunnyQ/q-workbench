//! `project source` runs once per keystroke, so these tests exercise the real binary:
//! the argv fast path, the silence requirement, and byte parity with the zsh script it
//! replaces.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn temporary_directory() -> PathBuf {
    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    let directory = std::env::temp_dir().join(format!(
        "workbench-project-source-cli-{}-{}-{}",
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

/// A registry with a hidden entry, an aliased entry, a used and an unused one.
fn write_fixture_registry(directory: &Path, home: &Path) -> PathBuf {
    let registry = directory.join("registry.json");
    let contents = format!(
        r#"{{"version":1,"generated_at":"2026-07-30T12:00:00Z","projects":{{
          "{home}/Alpha":{{"name":"Alpha","sources":["codex"],"aliases":[],"hidden":false}},
          "{home}/Aliased":{{"name":"Beta","sources":["claude","filesystem"],"aliases":["A","C"],"hidden":false}},
          "{home}/Hidden":{{"name":"Hidden","sources":["codex"],"hidden":true,"last_used_at":99}},
          "{home}/Used":{{"name":"Used","sources":["codex","manual"],"aliases":[],"hidden":false,"last_used_at":10}},
          "/opt/Outside":{{"name":"Outside","sources":["manual"],"hidden":false}}
        }}}}"#,
        home = home.display()
    );
    fs::write(&registry, contents).expect("write fixture registry");
    registry
}

fn run(registry: &Path, home: &Path, query: Option<&str>) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_workbench"));
    command.args(["project", "source"]);
    if let Some(query) = query {
        command.arg(query);
    }
    command
        .env("HOME", home)
        .env("Q_WORKBENCH_LOCAL_CONFIG", home.join("no-config.toml"))
        .env("Q_PROJECT_REGISTRY_FILE", registry)
        .output()
        .expect("run workbench project source")
}

#[test]
fn emits_the_registry_as_nul_delimited_records() {
    let directory = temporary_directory();
    let home = directory.join("home");
    let registry = write_fixture_registry(&directory, &home);

    let output = run(&registry, &home, None);

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 output"),
        format!(
            "\u{f024b}  Used\n   ~/Used\n   codex \u{b7} manual\t{home}/Used\0\
             \u{f024b}  Alpha\n   ~/Alpha\n   codex\t{home}/Alpha\0\
             \u{f024b}  Beta | A | C\n   ~/Aliased\n   claude \u{b7} filesystem\t{home}/Aliased\0\
             \u{f024b}  Outside\n   /opt/Outside\n   manual\t/opt/Outside\0",
            home = home.display()
        )
    );
    fs::remove_dir_all(&directory).expect("remove temporary directory");
}

/// The picker reloads this on every keystroke, so a failure must stay silent on both
/// channels — stdout would become an fzf row and stderr would corrupt the popup.
#[test]
fn a_missing_registry_exits_non_zero_without_writing_anything() {
    let directory = temporary_directory();
    let home = directory.join("home");
    let missing = directory.join("missing.json");

    let output = run(&missing, &home, None);

    assert!(!output.status.success());
    assert_eq!(output.stdout, Vec::<u8>::new());
    assert_eq!(output.stderr, Vec::<u8>::new());
    fs::remove_dir_all(&directory).expect("remove temporary directory");
}

/// `zoxide` is optional. With an empty PATH the spawn fails with `NotFound`, and the
/// registry rows must still come out with a zero exit.
#[test]
fn a_missing_zoxide_binary_is_not_an_error() {
    let directory = temporary_directory();
    let home = directory.join("home");
    let registry = write_fixture_registry(&directory, &home);

    let output = Command::new(env!("CARGO_BIN_EXE_workbench"))
        .args(["project", "source", "alp"])
        .env("HOME", &home)
        .env("PATH", directory.join("empty-path"))
        .env("Q_WORKBENCH_LOCAL_CONFIG", home.join("no-config.toml"))
        .env("Q_PROJECT_REGISTRY_FILE", &registry)
        .output()
        .expect("run workbench project source");

    assert!(output.status.success());
    assert_eq!(output.stdout.iter().filter(|byte| **byte == 0).count(), 4);
    assert_eq!(output.stderr, Vec::<u8>::new());
    fs::remove_dir_all(&directory).expect("remove temporary directory");
}
