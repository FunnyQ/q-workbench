use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// These tests run in parallel and each removes its own directory, so the name has to
/// be unique. The clock alone is not enough: macOS reports microseconds, and two tests
/// starting in the same microsecond would share a directory and delete each other's
/// files. The counter makes the name unique within the process regardless.
fn temporary_directory() -> std::path::PathBuf {
    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    let directory = std::env::temp_dir().join(format!(
        "workbench-config-migrate-cli-{}-{}-{}",
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

#[test]
fn write_refuses_then_force_overwrites_the_resolved_destination() {
    let directory = temporary_directory();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/config.zsh");
    let destination = directory.join("config.toml");
    let binary = env!("CARGO_BIN_EXE_workbench");

    let first = Command::new(binary)
        .args(["config", "migrate", "--from"])
        .arg(&fixture)
        .arg("--write")
        .env("HOME", directory.join("home"))
        .env("Q_WORKBENCH_LOCAL_CONFIG", &destination)
        .output()
        .expect("run first migration");
    assert!(first.status.success(), "{first:?}");
    assert_eq!(
        String::from_utf8(first.stdout)
            .expect("UTF-8 stdout")
            .trim(),
        destination.display().to_string()
    );

    let second = Command::new(binary)
        .args(["config", "migrate", "--from"])
        .arg(&fixture)
        .arg("--write")
        .env("HOME", directory.join("home"))
        .env("Q_WORKBENCH_LOCAL_CONFIG", &destination)
        .output()
        .expect("run second migration");
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("refusing to overwrite"));

    let forced = Command::new(binary)
        .args(["config", "migrate", "--from"])
        .arg(&fixture)
        .args(["--write", "--force"])
        .env("HOME", directory.join("home"))
        .env("Q_WORKBENCH_LOCAL_CONFIG", &destination)
        .output()
        .expect("run forced migration");
    assert!(forced.status.success(), "{forced:?}");

    fs::remove_dir_all(directory).expect("remove temporary directory");
}

#[test]
fn force_without_write_is_rejected() {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/config.zsh");
    let output = Command::new(env!("CARGO_BIN_EXE_workbench"))
        .args(["config", "migrate", "--from"])
        .arg(fixture)
        .arg("--force")
        .output()
        .expect("run migration");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--force requires --write"));
}

#[test]
fn a_missing_source_names_the_path_and_the_reason() {
    let output = Command::new(env!("CARGO_BIN_EXE_workbench"))
        .args(["config", "migrate", "--from", "/tmp/workbench-no-such.zsh"])
        .output()
        .expect("run migration");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    assert!(!output.status.success());
    assert!(stderr.contains("/tmp/workbench-no-such.zsh"), "{stderr}");
    assert!(stderr.contains("does not exist"), "{stderr}");
}

/// `Q_WORKBENCH_LOCAL_CONFIG` can point the destination anywhere, so `--force` must not
/// be able to write the migrated TOML over the `config.zsh` it was read from.
#[test]
fn write_refuses_a_destination_that_is_the_source() {
    let directory = temporary_directory();
    let source = directory.join("config.zsh");
    fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/config.zsh"),
        &source,
    )
    .expect("copy fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_workbench"))
        .args(["config", "migrate", "--from"])
        .arg(&source)
        .args(["--write", "--force"])
        .env("HOME", directory.join("home"))
        .env("Q_WORKBENCH_LOCAL_CONFIG", &source)
        .output()
        .expect("run migration");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    assert!(!output.status.success());
    assert!(
        stderr.contains("not a .toml file") || stderr.contains("migration source"),
        "{stderr}"
    );
    assert_eq!(
        fs::read_to_string(&source).expect("read source"),
        fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/config.zsh")
        )
        .expect("read fixture"),
        "the source file was modified"
    );

    fs::remove_dir_all(directory).expect("remove temporary directory");
}

#[test]
fn write_refuses_a_destination_that_is_not_named_toml() {
    let directory = temporary_directory();
    let destination = directory.join("config.zsh");
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/config.zsh");

    let output = Command::new(env!("CARGO_BIN_EXE_workbench"))
        .args(["config", "migrate", "--from"])
        .arg(&fixture)
        .args(["--write", "--force"])
        .env("HOME", directory.join("home"))
        .env("Q_WORKBENCH_LOCAL_CONFIG", &destination)
        .output()
        .expect("run migration");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a .toml file"));
    assert!(!destination.exists(), "the destination was written anyway");

    fs::remove_dir_all(directory).expect("remove temporary directory");
}

#[test]
fn help_warns_that_zsh_executes_the_source_file() {
    let output = Command::new(env!("CARGO_BIN_EXE_workbench"))
        .args(["config", "migrate", "--help"])
        .output()
        .expect("show help");
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 stdout");

    assert!(output.status.success());
    assert!(stdout.contains("executed by zsh"));
}
