use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const REGISTRY_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SshSource {
    Config,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshTarget {
    pub source: SshSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    pub last_used_at: Option<u64>,
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshRegistry {
    version: u8,
    pub targets: BTreeMap<String, SshTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRecord {
    pub target: String,
    pub hostname: String,
    pub user: String,
    pub aliases: Vec<String>,
}

// Config records are regenerated on every sync, so stale config entries must go.
// Manual records have no other source of truth and must survive reconciliation.
pub fn reconcile(
    mut targets: BTreeMap<String, SshTarget>,
    configured: Vec<ConfigRecord>,
) -> BTreeMap<String, SshTarget> {
    let configured_keys = configured
        .iter()
        .map(|record| record.target.as_str())
        .collect::<HashSet<_>>();
    targets.retain(|key, value| {
        value.source != SshSource::Config || configured_keys.contains(key.as_str())
    });

    for record in configured {
        let target = targets.entry(record.target).or_insert_with(|| SshTarget {
            source: SshSource::Config,
            hostname: None,
            user: None,
            aliases: None,
            last_used_at: None,
            hidden: false,
        });
        target.source = SshSource::Config;
        target.hostname = Some(record.hostname);
        target.user = Some(record.user);
        target.aliases = Some(record.aliases);
    }
    targets
}

pub fn sync(registry: &Path, config: &Path, history: &Path) -> Result<SshRegistry> {
    let existing = read_valid_registry(registry);
    let targets = match existing {
        Some(registry) => registry.targets,
        None => seed_targets(history_targets(history)?),
    };
    let registry_data = SshRegistry {
        version: REGISTRY_VERSION,
        targets: reconcile(targets, config_records(config)?),
    };
    write_registry(registry, &registry_data)?;
    Ok(registry_data)
}

pub fn list(registry: &Path, config: &Path, history: &Path) -> Result<Vec<u8>> {
    let registry = sync(registry, config, history)?;
    Ok(render_list(&registry.targets))
}

pub fn get(registry: &Path, config: &Path, history: &Path, target: &str) -> Result<String> {
    let registry = sync(registry, config, history)?;
    let target = registry
        .targets
        .get(target)
        .with_context(|| format!("SSH target not found: {target}"))?;
    let mut output = serde_json::to_string_pretty(target).context("serialize SSH target")?;
    output.push('\n');
    Ok(output)
}

pub fn use_target(registry: &Path, config: &Path, history: &Path, requested: &str) -> Result<()> {
    let mut registry_data = sync(registry, config, history)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_secs();
    apply_use(&mut registry_data.targets, requested, now);
    write_registry(registry, &registry_data)
}

pub fn remove(registry: &Path, config: &Path, history: &Path, requested: &str) -> Result<()> {
    let mut registry_data = sync(registry, config, history)?;
    apply_remove(&mut registry_data.targets, requested);
    write_registry(registry, &registry_data)
}

fn read_valid_registry(path: &Path) -> Option<SshRegistry> {
    let registry = fs::read_to_string(path).ok()?;
    let registry: SshRegistry = serde_json::from_str(&registry).ok()?;
    (registry.version == REGISTRY_VERSION).then_some(registry)
}

// Seed only when the registry is absent or invalid. Re-seeding every sync would
// restore history targets that the owner deliberately removed.
fn seed_targets(history: Vec<String>) -> BTreeMap<String, SshTarget> {
    history
        .into_iter()
        .map(|target| {
            (
                target,
                SshTarget {
                    source: SshSource::Manual,
                    hostname: None,
                    user: None,
                    aliases: None,
                    last_used_at: None,
                    hidden: false,
                },
            )
        })
        .collect()
}

// Collapse a unique user@hostname manual match into its config record. Otherwise,
// adding a host to SSH config would leave duplicate records after the next use.
fn apply_use(targets: &mut BTreeMap<String, SshTarget>, requested: &str, now: u64) {
    let target = resolve_alias(targets, requested);
    let (user, hostname) = target
        .split_once('@')
        .map_or(("", target.as_str()), |(user, hostname)| (user, hostname));
    let matches = targets
        .iter()
        .filter(|(_, value)| {
            value.source == SshSource::Config
                && value.hostname.as_deref() == Some(hostname)
                && (user.is_empty() || value.user.as_deref() == Some(user))
        })
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();

    if targets
        .get(&target)
        .is_some_and(|value| value.source == SshSource::Config)
    {
        stamp_target(targets, &target, now);
    } else if matches.len() == 1 {
        targets.remove(&target);
        stamp_target(targets, &matches[0], now);
    } else if let Some(existing) = targets.get_mut(&target) {
        existing.last_used_at = Some(now);
        existing.hidden = false;
    } else {
        targets.insert(
            target,
            SshTarget {
                source: SshSource::Manual,
                hostname: None,
                user: None,
                aliases: None,
                last_used_at: Some(now),
                hidden: false,
            },
        );
    }
}

fn resolve_alias(targets: &BTreeMap<String, SshTarget>, requested: &str) -> String {
    if targets.contains_key(requested) {
        return requested.to_owned();
    }
    targets
        .iter()
        .find(|(_, target)| {
            target
                .aliases
                .as_ref()
                .is_some_and(|aliases| aliases.iter().any(|alias| alias == requested))
        })
        .map_or_else(|| requested.to_owned(), |(key, _)| key.clone())
}

fn stamp_target(targets: &mut BTreeMap<String, SshTarget>, key: &str, now: u64) {
    if let Some(target) = targets.get_mut(key) {
        target.last_used_at = Some(now);
        target.hidden = false;
    }
}

// Config records are hidden because sync would recreate a deleted record. Manual
// records exist only here, so removal can delete them permanently.
fn apply_remove(targets: &mut BTreeMap<String, SshTarget>, requested: &str) {
    if targets
        .get(requested)
        .is_some_and(|target| target.source == SshSource::Config)
    {
        if let Some(target) = targets.get_mut(requested) {
            target.hidden = true;
        }
    } else {
        targets.remove(requested);
    }
}

fn render_list(targets: &BTreeMap<String, SshTarget>) -> Vec<u8> {
    let mut entries = targets
        .iter()
        .filter(|(_, target)| !target.hidden)
        .collect::<Vec<_>>();
    entries.sort_by(|(left_key, left), (right_key, right)| {
        match (left.last_used_at, right.last_used_at) {
            (Some(left), Some(right)) => right.cmp(&left),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => left_key.cmp(right_key),
        }
    });

    let mut output = Vec::new();
    for (key, target) in entries {
        match target.source {
            SshSource::Config => {
                let aliases = target
                    .aliases
                    .as_ref()
                    .map_or_else(|| key.to_string(), |aliases| aliases.join("  "));
                write!(
                    output,
                    "{aliases}\n{}@{}\n[config]\t{key}\0",
                    target.user.as_deref().unwrap_or_default(),
                    target.hostname.as_deref().unwrap_or_default()
                )
                .expect("writing to Vec cannot fail");
            }
            SshSource::Manual => {
                write!(output, "{key}\n[manual]\t{key}\0").expect("writing to Vec cannot fail");
            }
        }
    }
    output
}

fn write_registry(path: &Path, registry: &SshRegistry) -> Result<()> {
    super::project::write_json_atomically(path, registry)
}

struct SshOutput {
    success: bool,
    stdout: String,
}

trait SshRunner {
    fn resolve(&self, config: &Path, target: &str) -> Result<SshOutput>;
}

struct CommandSshRunner;

impl SshRunner for CommandSshRunner {
    fn resolve(&self, config: &Path, target: &str) -> Result<SshOutput> {
        let output = Command::new("ssh")
            .args(["-G", "-F"])
            .arg(config)
            .args(["--", target])
            .output()
            .context("run ssh -G")?;

        Ok(SshOutput {
            success: output.status.success(),
            stdout: String::from_utf8(output.stdout).context("parse ssh -G output")?,
        })
    }
}

pub fn config_records(config: &Path) -> Result<Vec<ConfigRecord>> {
    config_records_with_runner(config, &CommandSshRunner)
}

fn config_records_with_runner(config: &Path, runner: &impl SshRunner) -> Result<Vec<ConfigRecord>> {
    let contents = match fs::read_to_string(config) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).context("read SSH config"),
    };

    let mut groups = contents
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            if !fields.next()?.eq_ignore_ascii_case("host") {
                return None;
            }

            let aliases = fields
                .filter(|alias| !alias.contains(['*', '!', '?']))
                .map(str::to_owned)
                .collect::<Vec<_>>();
            (!aliases.is_empty()).then_some(aliases)
        })
        .collect::<Vec<_>>();

    groups.sort();
    groups.dedup();

    let mut records = Vec::new();
    for aliases in groups {
        let target = aliases[0].clone();
        let Ok(output) = runner.resolve(config, &target) else {
            continue;
        };
        if !output.success {
            continue;
        }

        let hostname = first_value(&output.stdout, "hostname").unwrap_or_default();
        let user = first_value(&output.stdout, "user").unwrap_or_default();
        records.push(ConfigRecord {
            target,
            hostname,
            user,
            aliases,
        });
    }

    Ok(records)
}

fn first_value(output: &str, expected_key: &str) -> Option<String> {
    // Effective config may repeat keys, and OpenSSH gives the first value precedence.
    output.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        (fields.next()? == expected_key).then(|| fields.next().unwrap_or_default().to_owned())
    })
}

pub fn history_targets(history: &Path) -> Result<Vec<String>> {
    let contents = match fs::read_to_string(history) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).context("read shell history"),
    };

    let mut occurrences = contents
        .lines()
        .enumerate()
        .filter_map(parse_history_line)
        .collect::<Vec<_>>();
    // Deduplication must happen after sorting or an older shell entry can hide the latest one.
    occurrences.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));

    let mut seen = HashSet::new();
    Ok(occurrences
        .into_iter()
        .filter_map(|(_, _, target)| seen.insert(target.clone()).then_some(target))
        .collect())
}

fn parse_history_line((index, line): (usize, &str)) -> Option<(u64, usize, String)> {
    let line = line.strip_prefix(": ")?;
    let (timing, command) = line.split_once(';')?;
    let (epoch, elapsed) = timing.split_once(':')?;
    let epoch = epoch.parse().ok()?;
    elapsed.parse::<u64>().ok()?;

    let command = command
        .strip_prefix("TERM=")
        .and_then(|command| command.split_once(' ').map(|(_, command)| command))
        .unwrap_or(command);
    let arguments = command.strip_prefix("ssh ")?.split_whitespace();
    let target = arguments.last()?.to_owned();

    is_history_target(&target).then_some((epoch, index, target))
}

fn is_history_target(target: &str) -> bool {
    !target.is_empty()
        && target
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "[]_.@-".contains(character))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        directory: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
            let directory =
                std::env::temp_dir().join(format!("workbench-ssh-{}-{id}", std::process::id()));
            fs::create_dir_all(&directory).unwrap();
            Self { directory }
        }

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.directory.join(name);
            fs::write(&path, contents).unwrap();
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.directory).unwrap();
        }
    }

    struct StubRunner {
        outputs: HashMap<String, SshOutput>,
    }

    impl SshRunner for StubRunner {
        fn resolve(&self, _config: &Path, target: &str) -> Result<SshOutput> {
            self.outputs
                .get(target)
                .map(|output| SshOutput {
                    success: output.success,
                    stdout: output.stdout.clone(),
                })
                .context("missing stub output")
        }
    }

    fn output(stdout: &str) -> SshOutput {
        SshOutput {
            success: true,
            stdout: stdout.to_owned(),
        }
    }

    #[test]
    fn parses_and_sorts_config_alias_groups() {
        let fixture = Fixture::new();
        let config = fixture.write(
            "config",
            "\
Host *
  ServerAliveInterval 30
Host web1 web-primary
Host !prod
Host staging? *.internal
host api api-primary
Host web1 web-primary
",
        );
        let runner = StubRunner {
            outputs: HashMap::from([
                ("api".to_owned(), output("hostname api.test\nuser deploy\n")),
                (
                    "web1".to_owned(),
                    output("hostname web.test\nuser webmaster\n"),
                ),
            ]),
        };

        let records = config_records_with_runner(&config, &runner).unwrap();

        assert_eq!(
            records,
            vec![
                ConfigRecord {
                    target: "api".to_owned(),
                    hostname: "api.test".to_owned(),
                    user: "deploy".to_owned(),
                    aliases: vec!["api".to_owned(), "api-primary".to_owned()],
                },
                ConfigRecord {
                    target: "web1".to_owned(),
                    hostname: "web.test".to_owned(),
                    user: "webmaster".to_owned(),
                    aliases: vec!["web1".to_owned(), "web-primary".to_owned()],
                },
            ]
        );
    }

    #[test]
    fn first_ssh_value_wins_when_keys_repeat() {
        let fixture = Fixture::new();
        let config = fixture.write("config", "Host web\n");
        let runner = StubRunner {
            outputs: HashMap::from([(
                "web".to_owned(),
                output("hostname first.test\nhostname second.test\nuser deploy\n"),
            )]),
        };

        let records = config_records_with_runner(&config, &runner).unwrap();

        assert_eq!(records[0].hostname, "first.test");
    }

    #[test]
    fn skips_failed_ssh_resolution() {
        let fixture = Fixture::new();
        let config = fixture.write("config", "Host broken\nHost working\n");
        let runner = StubRunner {
            outputs: HashMap::from([
                (
                    "broken".to_owned(),
                    SshOutput {
                        success: false,
                        stdout: String::new(),
                    },
                ),
                (
                    "working".to_owned(),
                    output("hostname working.test\nuser deploy\n"),
                ),
            ]),
        };

        let records = config_records_with_runner(&config, &runner).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].target, "working");
    }

    #[test]
    fn parses_history_targets_most_recent_first() {
        let fixture = Fixture::new();
        let history = fixture.write(
            "history",
            "\
: 100:0;ssh old
: 400:1;TERM=xterm-256color ssh -p 22 web
: 300:2;ssh deploy@example.com
: 500:0;ssh old
: 600:0;ssh \"bad target\"
: 550:0;ssh admin@[server]
plain ssh ignored
",
        );

        assert_eq!(
            history_targets(&history).unwrap(),
            vec!["admin@[server]", "old", "web", "deploy@example.com"]
        );
    }

    #[test]
    fn missing_files_return_empty_lists() {
        let fixture = Fixture::new();
        let missing = fixture.directory.join("missing");

        assert!(config_records(&missing).unwrap().is_empty());
        assert!(history_targets(&missing).unwrap().is_empty());
    }

    fn manual(last_used_at: Option<u64>) -> SshTarget {
        SshTarget {
            source: SshSource::Manual,
            hostname: None,
            user: None,
            aliases: None,
            last_used_at,
            hidden: false,
        }
    }

    fn configured(
        hostname: &str,
        user: &str,
        aliases: &[&str],
        last_used_at: Option<u64>,
    ) -> SshTarget {
        SshTarget {
            source: SshSource::Config,
            hostname: Some(hostname.to_owned()),
            user: Some(user.to_owned()),
            aliases: Some(aliases.iter().map(|alias| (*alias).to_owned()).collect()),
            last_used_at,
            hidden: false,
        }
    }

    #[test]
    fn reconciliation_drops_stale_config_and_preserves_manual_targets() {
        let existing = BTreeMap::from([
            (
                "changed".to_owned(),
                configured("old.example", "old-user", &["changed"], Some(10)),
            ),
            (
                "removed".to_owned(),
                configured("removed.example", "deploy", &["removed"], None),
            ),
            ("manual".to_owned(), manual(Some(20))),
        ]);
        let records = vec![
            ConfigRecord {
                target: "changed".to_owned(),
                hostname: "new.example".to_owned(),
                user: "new-user".to_owned(),
                aliases: vec!["changed".to_owned(), "changed-alt".to_owned()],
            },
            ConfigRecord {
                target: "added".to_owned(),
                hostname: "added.example".to_owned(),
                user: "deploy".to_owned(),
                aliases: vec!["added".to_owned()],
            },
        ];

        let reconciled = reconcile(existing, records);

        assert!(!reconciled.contains_key("removed"));
        assert_eq!(reconciled["manual"], manual(Some(20)));
        assert_eq!(
            reconciled["changed"].hostname.as_deref(),
            Some("new.example")
        );
        assert_eq!(reconciled["changed"].last_used_at, Some(10));
        assert_eq!(reconciled["added"].last_used_at, None);
    }

    #[test]
    fn sync_seeds_only_absent_or_invalid_registries() {
        for (name, initial) in [
            ("absent", None),
            ("invalid", Some("{not json}\n")),
            (
                "valid",
                Some(
                    "{\n  \"version\": 1,\n  \"targets\": {\n    \"kept\": {\n      \"source\": \"manual\",\n      \"last_used_at\": null,\n      \"hidden\": false\n    }\n  }\n}\n",
                ),
            ),
        ] {
            let fixture = Fixture::new();
            let registry = fixture.directory.join(name);
            if let Some(contents) = initial {
                fs::write(&registry, contents).unwrap();
            }
            let history = fixture.write("history", ": 100:0;ssh seeded\n");
            let config = fixture.directory.join("missing-config");

            let synced = sync(&registry, &config, &history).unwrap();

            assert_eq!(
                synced.targets.contains_key("seeded"),
                name != "valid",
                "{name}"
            );
            assert_eq!(synced.targets.contains_key("kept"), name == "valid", "{name}");
        }
    }

    #[test]
    fn use_resolves_alias_and_collapses_a_unique_manual_target() {
        let mut targets = BTreeMap::from([
            (
                "server".to_owned(),
                configured("server.example", "deploy", &["server", "server-alt"], None),
            ),
            ("deploy@server.example".to_owned(), manual(None)),
        ]);

        apply_use(&mut targets, "deploy@server.example", 100);

        assert!(!targets.contains_key("deploy@server.example"));
        assert_eq!(targets["server"].last_used_at, Some(100));
        targets.get_mut("server").unwrap().hidden = true;
        apply_use(&mut targets, "server-alt", 200);
        assert_eq!(targets["server"].last_used_at, Some(200));
        assert!(!targets["server"].hidden);
    }

    #[test]
    fn use_keeps_manual_target_when_two_config_targets_match() {
        let mut targets = BTreeMap::from([
            (
                "first".to_owned(),
                configured("shared.example", "deploy", &["first"], None),
            ),
            (
                "second".to_owned(),
                configured("shared.example", "deploy", &["second"], None),
            ),
            ("deploy@shared.example".to_owned(), manual(None)),
        ]);

        apply_use(&mut targets, "deploy@shared.example", 100);

        assert_eq!(targets["deploy@shared.example"].last_used_at, Some(100));
        assert_eq!(targets["first"].last_used_at, None);
        assert_eq!(targets["second"].last_used_at, None);
    }

    #[test]
    fn use_inserts_a_brand_new_manual_target() {
        let mut targets = BTreeMap::new();

        apply_use(&mut targets, "new@example.com", 123);

        assert_eq!(
            targets["new@example.com"],
            SshTarget {
                source: SshSource::Manual,
                hostname: None,
                user: None,
                aliases: None,
                last_used_at: Some(123),
                hidden: false,
            }
        );
    }

    #[test]
    fn remove_hides_config_and_deletes_manual_targets() {
        let mut targets = BTreeMap::from([
            (
                "configured".to_owned(),
                configured("host", "user", &["configured"], None),
            ),
            ("manual".to_owned(), manual(None)),
        ]);

        apply_remove(&mut targets, "configured");
        apply_remove(&mut targets, "manual");

        assert!(targets["configured"].hidden);
        assert!(!targets.contains_key("manual"));
    }

    #[test]
    fn list_is_nul_delimited_and_uses_the_contract_sort_and_layout() {
        let targets = BTreeMap::from([
            ("z-used".to_owned(), manual(Some(20))),
            (
                "config".to_owned(),
                configured(
                    "config.example",
                    "deploy",
                    &["config", "config-alt"],
                    Some(10),
                ),
            ),
            ("b-never".to_owned(), manual(None)),
            ("a-never".to_owned(), manual(None)),
        ]);

        assert_eq!(
            render_list(&targets),
            b"z-used\n[manual]\tz-used\0config  config-alt\ndeploy@config.example\n[config]\tconfig\0a-never\n[manual]\ta-never\0b-never\n[manual]\tb-never\0"
        );
    }

    #[test]
    fn list_filters_hidden_targets() {
        let mut hidden_manual = manual(Some(30));
        hidden_manual.hidden = true;
        let mut hidden_config = configured("hidden.example", "deploy", &["hidden"], Some(20));
        hidden_config.hidden = true;
        let targets = BTreeMap::from([
            ("hidden-config".to_owned(), hidden_config),
            ("hidden-manual".to_owned(), hidden_manual),
            ("visible".to_owned(), manual(Some(10))),
        ]);

        assert_eq!(render_list(&targets), b"visible\n[manual]\tvisible\0");
    }

    #[test]
    fn get_returns_zsh_shaped_json_and_errors_for_a_missing_target() {
        let fixture = Fixture::new();
        let registry = fixture.write(
            "registry.json",
            "{\n  \"version\": 1,\n  \"targets\": {\n    \"server\": {\n      \"source\": \"config\",\n      \"hostname\": \"server.example\",\n      \"user\": \"deploy\",\n      \"aliases\": [\n        \"server\",\n        \"server-alt\"\n      ],\n      \"last_used_at\": null,\n      \"hidden\": false\n    }\n  }\n}\n",
        );
        let config = fixture.write(
            "config",
            "Host server server-alt\n  HostName server.example\n  User deploy\n",
        );
        let missing_history = fixture.directory.join("missing-history");

        assert_eq!(
            get(&registry, &config, &missing_history, "server").unwrap(),
            "{\n  \"source\": \"config\",\n  \"hostname\": \"server.example\",\n  \"user\": \"deploy\",\n  \"aliases\": [\n    \"server\",\n    \"server-alt\"\n  ],\n  \"last_used_at\": null,\n  \"hidden\": false\n}\n"
        );
        assert_eq!(
            get(&registry, &config, &missing_history, "missing")
                .unwrap_err()
                .to_string(),
            "SSH target not found: missing"
        );
    }

    #[test]
    fn registry_json_is_byte_identical_to_zsh_output() {
        let fixture = Fixture::new();
        let path = fixture.directory.join("registry.json");
        write_registry(
            &path,
            &SshRegistry {
                version: REGISTRY_VERSION,
                targets: BTreeMap::from([
                    (
                        "configured".to_owned(),
                        configured(
                            "configured.example",
                            "deploy",
                            &["configured", "configured-alt"],
                            Some(42),
                        ),
                    ),
                    ("manual".to_owned(), manual(None)),
                ]),
            },
        )
        .unwrap();

        assert_eq!(
            fs::read(path).unwrap(),
            b"{\n  \"version\": 1,\n  \"targets\": {\n    \"configured\": {\n      \"source\": \"config\",\n      \"hostname\": \"configured.example\",\n      \"user\": \"deploy\",\n      \"aliases\": [\n        \"configured\",\n        \"configured-alt\"\n      ],\n      \"last_used_at\": 42,\n      \"hidden\": false\n    },\n    \"manual\": {\n      \"source\": \"manual\",\n      \"last_used_at\": null,\n      \"hidden\": false\n    }\n  }\n}\n"
        );
    }
}
