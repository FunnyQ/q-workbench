# Shared context

> All tasks reference this. Decisions here override anything inferred from the codebase.

## Project at a glance

`q.workbench` is a Herdr plugin (plugin id `q.workbench`) living at
`/Users/funnyq/Projects/q-lab/herdr-workbench`. It ships terminal-multiplexer actions
for one user, Q: launching AI coding agents into a 3-pane layout, fuzzy-picking
projects and SSH targets, and restarting a stuck agent in place.

It is currently 14 zsh scripts under `scripts/` plus 9 standalone zsh tests under
`tests/`. This plan replaces all of them with one Rust binary. The zsh version stays
on disk and stays wired into `herdr-plugin.toml` until the cutover commit at the very
end, so it is always available as the behavioural reference. That commit contains the
manifest flip and the deletion together, so it can be reverted as one unit;
documentation lands in a second commit immediately after.

## Tech stack

- **Language**: Rust, edition 2021. Toolchain 1.97.1 via mise.
- **Crates**: `clap` (derive), `serde` (derive), `serde_json`, `toml`, `anyhow`, `libc`,
  `time`.
  `libc` is needed for three things and nothing else: `setsid()` when detaching the
  restart worker, `kill()` with a negative pgid to signal a process group, and signal
  handling in the SSH session wrapper.
  `time` (with the `formatting` feature) exists for one reason: the project registry's
  `generated_at` field is a formatted UTC calendar timestamp, and the standard library
  has no calendar conversion. Unix-epoch values such as `last_used_at` need it — use
  `SystemTime::UNIX_EPOCH.elapsed()` for those.
- **No async runtime.** The Herdr socket serves exactly one request per connection, so
  a blocking `std::os::unix::net::UnixStream` is sufficient. Do not add `tokio`.
- **External processes that stay**: `gum` (menus), `fzf` (pickers), `yazi` (file pane),
  `zoxide`, `git`, `ssh`, `trash`. `jq` is dropped entirely.
- **The `herdr` CLI is never spawned from `src/`.** One scoped exemption exists outside
  the code: five `[[actions]]` in `herdr-plugin.toml` keep
  `command = ["herdr", "plugin", "pane", "open", …]`, because that is Herdr's own
  mechanism for opening a plugin popup and Herdr — not this plugin — runs it.
  Proxying it through the binary was rejected: `plugin.pane.open` exists as a socket
  method, but the binary would first have to learn its own plugin id (`q.workbench`
  installed, `q.workbench-dev` linked) and no environment variable carrying that was
  verified.
- **Platform**: macOS arm64 only. The manifest declares `platforms = ["macos"]`.

## Build and artifact layout

- Source in `src/`. `cargo build --release` writes to `target/` (gitignored).
- `scripts/build.zsh` is the only sanctioned build path: it runs
  `cargo build --release` and copies the artifact to `bin/workbench`.
- **`bin/workbench` is committed to git.** `herdr plugin install FunnyQ/q-workbench`
  performs a real `git clone`, so a committed binary arrives with no install hook.
- Because the artifact is committed, an edit to `src/` does **not** take effect until
  `scripts/build.zsh` is rerun. This replaces the old "edit a script, next invocation
  picks it up" workflow and must be documented.

## Code style

- `rustfmt` defaults. `cargo clippy -- -D warnings` must be clean.
- Q has no prior Rust experience. **Favour straightforward, owned-data code over
  clever abstractions.** Prefer `String` and `.clone()` to lifetime parameters; at
  these runtimes the cost is unmeasurable.
- `anyhow::Result` for fallible functions; `.context("…")` on every I/O or parse
  boundary so the eventual notification carries a concrete cause.
- Comments explain **why**, not what. The zsh original is dense with ordering traps,
  TTY quirks and git-worktree constraints — carry those comments across rather than
  dropping them. Comments in English.
- One term per thing. Do not rename a concept in the port (`worktree`, `harness`,
  `usage`, `label`, `target`, `registry` all keep their meaning).

## File / directory layout

```
Cargo.toml
bin/workbench          # committed release artifact
scripts/build.zsh      # the build + copy script
dev/                   # dev-only linked plugin (see below)
src/main.rs            # clap subcommand dispatch, nothing else
src/config.rs          # TOML config, replaces scripts/config.zsh
src/notify.rs          # notification.show helper
src/shell.rs           # shell_quote()
src/state.rs           # per-pane last harness/model
src/herdr/mod.rs       # HerdrClient trait, SocketClient, FakeClient
src/herdr/types.rs     # serde request/response structs
src/registry/project.rs
src/registry/ssh.rs
src/flows/agent.rs
src/flows/picker.rs
src/flows/restart.rs
src/flows/dashboard.rs
```

New modules go under the matching directory. Unit tests live in a `#[cfg(test)] mod
tests` block at the bottom of the file they test. Integration tests that need a real
socket go in `tests/` at the crate root (Rust integration tests, not the old zsh ones).

## Subcommand surface

`clap` derive, one binary named `workbench`:

```
workbench agent popup [--worktree]
workbench agent launch <pane_id> [--tab <id>] [--usage <label>] [--worktree] [--no-layout] [--restart]
workbench agent inject <pane_id> [--tab <id>] [--usage <label>] [--worktree]
workbench agent restart
workbench agent restart-worker --pane <pane_id>   # hidden; spawned by `agent restart`
workbench project pick
workbench project source [query]
workbench project scan|rescan|update
workbench project use <path>
workbench project edit <path>
workbench ssh pick
workbench ssh sync|list
workbench ssh get|use|remove <target>
workbench ssh edit [target]
workbench ssh session <target> <tab_id>
workbench dashboard
workbench config migrate [--from <path>] [--write] [--force]
workbench herdr ping
```

`agent launch --restart` is hidden. It is set only by the restart worker's injected
command and marks the launch as a restart; `--no-layout` is a public option and does
**not** imply it.

`ssh edit` takes an **optional** target. The picker's `ctrl-i` binding passes `{2}`,
which is empty when nothing is selected, so a required argument would make clap reject
the call before the flow could report it.

`config migrate` prints TOML to stdout by default; `--write` installs it at the
resolved config path, refusing to overwrite unless `--force` is also given. `--from`
overrides the source `config.zsh`.

The old positional-slot convention (`'' '' ''`) is gone — named flags replace it.

## Config

The TOML config lives at
`${XDG_CONFIG_HOME:-$HOME/.config}/herdr/plugins/config/q.workbench/config.toml`.

That literal path is used directly. There is **no** `herdr plugin config-dir`
shellout: it is the single method with no socket equivalent, and the zsh version
already carried the same literal as a fallback.

Precedence: user TOML file → environment variable → built-in default.

`Q_WORKBENCH_LOCAL_CONFIG` still overrides the resolved path, and tests must set it
(to a nonexistent path or `/dev/null`) so a developer's real config cannot leak in.

## External process contract

`gum` writes its selection to **stdout** and draws its UI on the controlling TTY.
`std::process::Command::output()` captures stdout while the UI still renders — this is
exactly the `$(gum choose …)` contract the zsh version relies on. A non-zero exit
means the user cancelled; treat it as "abort this flow quietly", never as an error to
notify about.

`fzf` is invoked the same way. Its `--bind` expressions call back into `workbench`
itself, so build them from `std::env::current_exe()`, not a hardcoded path.

## Commit & branching style

- Base branch: `main`. Chronicle is configured `whole-repo`, tag `v{version}`, with
  `herdr-plugin.toml` as the version file.
- Use `chronicle:commit` for commits. Do not craft commits by hand.
- `Cargo.toml` must be added to `.chronicle/release.json` `versionFiles` during
  cutover so the two versions cannot drift.

## Verification baseline

- `cargo test` — all tests.
- `cargo clippy -- -D warnings` — clean.
- `zsh scripts/build.zsh` — produces `bin/workbench`.
- The old suite still runs during development:
  `for t in tests/*.test.zsh; do zsh "$t" || break; done`. It is deleted at cutover,
  not before.

## Dev-plugin harness

Cutover is a big-bang flip, so the installed `q.workbench` keeps running zsh
throughout development. To drive Rust flows for real, a second plugin is linked:

- `dev/herdr-plugin.toml` declares plugin id `q.workbench-dev` with the same actions
  and panes, pointing at `./run.zsh`.
- `dev/run.zsh` is a two-line shim: `exec "${0:A:h:h}/bin/workbench" "$@"`.
- `herdr plugin link dev/` registers it; `herdr plugin unlink q.workbench-dev` removes
  it. Herdr resolves a relative `command[0]` against the plugin root (verified).
- `dev/` is committed during development and **deleted at cutover**. Its whole purpose
  is exercising Rust flows while the installed plugin still runs zsh; once the manifest
  points at the binary, linking the repo itself does the same job. Leaving it would also
  break the plan's goal of exactly one remaining zsh script.

## Safety rules while working in this repo

- **Never drive a live pane.** Any experiment that sends input, kills a process group,
  or splits panes happens in a scratch tab created for the purpose and closed after.
- Use `trash`, never `rm` — including in test cleanup.
- Ask before anything irreversible or that costs money.
