use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture() -> PathBuf {
    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "workbench-project-registry-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_empty_registry(path: &Path) {
    fs::write(
        path,
        b"{\"version\":1,\"generated_at\":\"old\",\"projects\":{}}\n",
    )
    .unwrap();
}

fn run(home: &Path, registry: &Path, root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_workbench"))
        .args(args)
        .env("HOME", home)
        .env("Q_WORKBENCH_LOCAL_CONFIG", home.join("missing-config.toml"))
        .env("Q_PROJECT_REGISTRY_FILE", registry)
        .env("Q_PROJECTS_ROOT", root)
        .output()
        .unwrap()
}

#[test]
fn edit_relative_path_uses_exact_stderr_and_failure_exit() {
    let directory = fixture();
    let registry = directory.join("registry.json");
    let root = directory.join("projects");
    fs::create_dir(&root).unwrap();
    write_empty_registry(&registry);

    let output = run(
        &directory,
        &registry,
        &root,
        &["project", "edit", "relative"],
    );

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        b"project-registry: absolute project path is required\n"
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn update_uses_real_stdout_and_success_exit() {
    let directory = fixture();
    let registry = directory.join("registry.json");
    let root = directory.join("projects");
    fs::create_dir(&root).unwrap();
    write_empty_registry(&registry);

    let output = run(&directory, &registry, &root, &["project", "update"]);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        output.stdout,
        format!("project-registry: updated {}\n", registry.display()).as_bytes()
    );
    assert_eq!(output.stderr, b"");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn unnamed_failure_uses_subcommand_path_on_real_stderr() {
    let directory = fixture();
    let registry = directory.join("registry.json");
    let missing_root = directory.join("projects");
    fs::create_dir(&missing_root).unwrap();
    write_empty_registry(&registry);
    fs::write(directory.join("missing-config.toml"), "not = [valid").unwrap();

    let output = run(&directory, &registry, &missing_root, &["project", "update"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"");
    assert!(output.stderr.starts_with(b"project update: "));
    fs::remove_dir_all(directory).unwrap();
}
