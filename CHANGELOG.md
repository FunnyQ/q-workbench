# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.10.1] - 2026-09-23

_tracks tag `v0.10.1`_

### Fixed
- Opening a project into a new workspace could start its agent launcher
  before the workspace's terminal had resized to its on-screen tab
  dimensions, so the launcher's banner centered for the wrong width and
  never redrew. The new workspace is now focused before the launcher is
  injected, so it measures the real terminal size.

## [0.10.0] - 2026-09-23

_tracks tag `v0.10.0`_

### Added
- The model menu now takes an effort level per model: left/right arrows step
  the highlighted row through its levels while up/down still moves between
  models, and each row remembers its own choice. Configure it with an agent's
  `efforts` list and `effort_args` template (using an `{effort}`
  placeholder), then opt an option in with a default `effort`; an option can
  override the level list for models that support a different set via its
  own `efforts`. An option with no `effort` shows none, so existing configs
  are unaffected until you opt in.

### Changed
- The built-in OpusPlan option now sets `effort = "medium"` instead of
  passing `--effort medium` in `args`; the resulting command line is
  unchanged. If your own config already lists `efforts` for an option whose
  `args` still hard-codes an effort flag, move that flag into `effort` too —
  otherwise it is passed twice.

## [0.9.3] - 2026-09-23

_tracks tag `v0.9.3`_

### Fixed
- The SSH picker no longer closes immediately on open for users whose shell
  history contains non-ASCII bytes. zsh metafies those bytes, so history
  files are rarely valid UTF-8; the picker now decodes them leniently
  instead of aborting, since SSH targets themselves are always ASCII.

## [0.9.2] - 2026-09-17

_tracks tag `v0.9.2`_

### Changed
- No user-facing behavior change. This release confirms compatibility with
  Herdr 0.9.1 (the socket protocol stays at 22) and regenerates the
  committed protocol baseline, which now tracks 17 response types instead
  of 8. A test-fixture typo that used the event topic name `tab.created`
  instead of the real result type `tab_created` is also fixed.

## [0.9.1] - 2026-09-14

_tracks tag `v0.9.1`_

### Fixed
- Opening a second agent tab under the same usage label (e.g. "discuss") no
  longer fails and closes the popup tab. Herdr rejected the reused agent name
  with `agent_name_taken`; starting the pane now retries with a numbered name
  (`discuss-2`, `discuss-3`, ...) instead of giving up.

## [0.9.0] - 2026-09-09

_tracks tag `v0.9.0`_

### Added
- Restarting an agent now asks whether to resume its existing session, start
  fresh, or cancel. The three rows always draw, since the popup closes before
  it could check whether a session id exists; a detached worker resolves it
  afterwards, preferring Herdr's own reported session, then falling back to a
  sweep of the harness's own session files bounded by the launch time. A
  Resume that ends up with no usable id — none found, one that is only a
  transcript path, or a harness with no resume argument — falls back to a
  fresh start and says why in a notification instead of silently starting
  over.
- Every agent pane now spawns a detached reporter that finds the session file
  the agent wrote and reports it to Herdr, so Herdr's own session snapshot
  stays current and restart has an id to resume even before the sweep above
  runs. It matches by the pane's cwd and launch time, so two agents sharing a
  cwd can still be credited each other's session.
- Starting a pane through Herdr's `agent.start` now retries while a
  just-created pane reports itself busy loading its shell profile, and sends
  a slugified agent name instead of the pane's own label, which Herdr was
  rejecting outright for containing glyphs, capitals, or spaces.
- Agents can now declare an optional `kind` in config, which lets the plugin
  start that pane through `agent.start` — making it an agent Herdr can see —
  instead of typing its command into the shell. An agent whose `command`
  isn't just the kind's own executable, or an option that overrides it, keeps
  the old typed-argv start.

### Changed
- The popup now builds a whole tab with one atomic `layout.apply` call instead
  of an incremental split-and-rename loop. A failed build leaves nothing
  behind, and the new tab is focused as soon as it exists rather than after
  every pane's agent has started. The in-pane `agent launch` path is
  unaffected: `layout.apply` would destroy the launcher's own pane before its
  `exec` ever ran.
- The plugin now requires a Herdr new enough to speak protocol v22
  (`MINIMUM_PROTOCOL` raised from 17), the version that added the three
  methods above.

## [0.8.1] - 2026-08-21

_tracks tag `v0.8.1`_

### Changed
- The new-tab menu now lists layouts in the order `[[tab_layouts]]` declares
  them, instead of always pushing `default_tab_layout` to the top. That
  setting still picks which layout launches when `--layout` is omitted; it
  no longer reorders the menu itself. The shipped config keeps its default
  layout first, so the cursor still lands on it by default.

## [0.8.0] - 2026-08-19

_tracks tag `v0.8.0`_

### Added
- A new "project review" popup action registers or refreshes the project
  registry without a shell. It picks scan or rescan automatically depending
  on whether the registry file already exists, and reports the result
  ("Registered N projects.") through a notification since a popup closes on
  exit and can't rely on stderr being seen.

### Changed
- README's Registries section now leads with "nothing to run after install,"
  moves the CLI walkthrough into its own "From a shell" subsection, and
  resolves the plugin's binary path via `herdr plugin list --plugin
  q.workbench --json` instead of an assumed local path.

### Fixed
- Release version bumps now also update `Cargo.lock`. Previously it was
  left at the old version and cargo would silently rewrite it into the next
  unrelated commit; `Cargo.lock` is caught up to the current version as
  part of this fix.
- Rebuilding `bin/workbench` no longer gets SIGKILLed by a stale macOS code
  signature cached against the binary's inode. The build script now writes
  to a temp file and swaps it into place instead of overwriting in place.

## [0.7.0] - 2026-08-13

_tracks tag `v0.7.0`_

### Added
- The project picker now surfaces a third source: projects living under
  `projects_root` that aren't in the registry or in zoxide yet. Type two
  characters and it appends them after the registry and zoxide rows, deduped
  against both, so an unregistered checkout shows up without needing a bulk
  import first. Being swept up does not register a project — picking it does,
  so the registry still holds only what you actually open.
- A new `project_markers` config key (default `package.json`, `Gemfile`,
  `Cargo.toml`, `CLAUDE.md`) lets the sweep recognize a project by more than
  just `.git`. Set it to `[]` to fall back to `.git`-only detection.

### Changed
- The picker's filesystem sweep now stops descending as soon as it finds a
  project, instead of continuing to walk every subdirectory underneath it.
  On a 133-checkout tree this cut the per-keystroke sweep from roughly
  1295ms to 14.6ms, at the cost of missing checkouts nested inside another
  checkout. The exhaustive sweep used by `project update` and the registry
  is unchanged.
- The picker's fzf border label now reads "type to add zoxide + projects
  root", reflecting the new filesystem source.

## [0.6.3] - 2026-08-10

_tracks tag `v0.6.3`_

### Fixed
- SSH sessions that dropped, were interrupted, or exited with a non-zero
  status sank to the bottom of the SSH picker, because a target was only
  stamped as used after a clean exit. Usage is now stamped at launch, so the
  target you connected to most recently sorts first regardless of how the
  session ended. Stamping stays best-effort and never affects `ssh`'s own
  exit status.

## [0.6.2] - 2026-08-05

_tracks tag `v0.6.2`_

### Fixed
- Every workbench action was failing with a "Workbench needs rebuilding"
  notification after upgrading to Herdr 0.8.0, because the plugin's
  protocol guard rejected any protocol version other than the exact one it
  was built against. The guard now accepts any protocol at or above the
  oldest version the plugin has been verified against, so newer Herdr
  releases work without a plugin update as long as they stay compatible.
  The notification now only fires when Herdr's protocol is genuinely too
  old, and it names the required minimum so it's clear what to update.

## [0.6.1] - 2026-08-04

_tracks tag `v0.6.1`_

### Added
- Tab layouts can now declare zero, one, or multiple agent panes. Each
  unpinned agent pane runs its own harness and model menu, in the order the
  panes are configured, instead of the launcher being restricted to a
  single agent pane at the layout's root.
- The new-tab menu's blank-tab row is now a real layout you can customize:
  a config layout can replace the built-in blank body while still keeping
  its place as the menu's last entry.
- Agent restart now remembers which pane an agent was launched in, so
  restarting targets the correct pane instead of assuming the first agent
  pane. Pre-upgrade restart state is discarded rather than silently reused
  under the new format.

### Changed
- Menu rows that open a further prompt (harness, model, layout choices) are
  now marked with an ellipsis, making it clearer which selections lead to
  another step before anything launches.

## [0.6.0] - 2026-08-03

_tracks tag `v0.6.0`_

### Added
- New `tab new` Herdr action opens a gum menu of every configured tab layout,
  so any layout added to `[[tab_layouts]]` is reachable from a single
  keybinding without a matching manifest action or extra rebuild.
  `default_tab_layout` is listed first, the rest keep config order, and a
  config with only one layout skips the menu and opens it directly.
- `[[tab_layouts]]` entries can now set optional `label` and `icon` keys,
  rendered the same way agent labels are, falling back to the layout's name.
  Config load now rejects an empty label/icon or two layouts that render to
  the same menu label, naming the offending layout.

### Changed
- Menu-drawing code shared by the agent and layout pickers moved into its own
  module, so both menus stay visually consistent as new pickers are added.

## [0.5.0] - 2026-08-03

_tracks tag `v0.5.0`_

### Added
- New `[[tab_layouts]]` and `[[agents]]` TOML sections replace the plugin's
  configuration surface. A layout describes a tab's panes declaratively and the
  launcher builds them, instead of five hardcoded Herdr socket calls. Adding or
  reordering harnesses and model options no longer requires a recompile.
- Omitting a choice in a layout still prompts for it at launch, so a layout that
  pins nothing reproduces the original popup menus, while a fully-pinned layout
  opens straight into the agent with no menus at all.
- New `--layout <name>` flag reaches non-default layouts from both the CLI and
  `herdr-plugin.toml` action bindings.
- Agent state moves to v2 (`STATE_VERSION 2`), keyed on stable config ids instead
  of rendered menu labels. Restarting a fully-pinned tab now asks nothing, and an
  older v1 state record is discarded rather than misread against the new schema.
- Every configuration error is now caught by `Config::load()` before the first
  socket call, with 25 distinct rejection branches each covered by its own test.

### Changed
- The CCR harness's special-cased launch behavior is now an ordinary agent option
  with a command override, removing a hardcoded branch from the launch path.

### Removed
- **BREAKING:** The five flat config fields — `order`, `models`, `model_args`,
  `claude_extra_args`, `codex_extra_args` — are gone, replaced by the
  `[[tab_layouts]]` and `[[agents]]` sections. There is no automatic migration;
  `config.example.toml` is the executable specification for the new schema.
- Removed `workbench config migrate` and the zsh-config migration surface it
  supported. Existing configs must be rewritten by hand against the new schema.
- Removed the `Q_AGENT_MODEL_ORDER`, `Q_AGENT_MODELS`, `Q_AGENT_MODEL_ARGS`,
  `Q_CLAUDE_EXTRA_ARGS`, and `Q_CODEX_EXTRA_ARGS` environment-variable overrides
  with the settings they belonged to. Path settings and `Q_DASHBOARD_WORKSPACE`
  keep their environment overrides.

## [0.3.0] - 2026-08-02

_tracks tag `v0.3.0`_

### Added
- New `workbench pane even` command (Herdr action `even-out-panes`, default
  keybinding `prefix+e`) evens out a pane's split ratios. Splitting a pane twice
  commonly leaves widths like 50/25/25 instead of even thirds; this walks the
  maximal same-direction split chain containing the target pane and rebalances it,
  while leaving orthogonal nested splits (for example a Files/terminal stack)
  untouched.
- Built on two Herdr socket RPCs, `layout.export` and `layout.set_split_ratio`,
  discovered via `herdr api schema --json` and verified against a live Herdr
  session.
- README documents the new action and its default keybinding.

## [0.2.0] - 2026-08-01

_tracks tag `v0.2.0`_

### Added
- Rewrote the plugin from zsh scripts to a single Rust binary (`bin/workbench`),
  covering agent launch, project/SSH pickers, and in-place restart. Behaviour was
  checked against 64 parity clauses from the original zsh implementation before
  cutover.
- Configuration now loads from TOML with environment-variable overrides; run
  `workbench config migrate` to convert an existing zsh config.
- Terminal and picker failures report through stderr with consistent formatting,
  reserving popup notifications for flows that need them.
- Added a "use last" menu entry that repeats the previous agent/model combination
  without walking the menus again.

### Fixed
- Popup menus now render at the pane's actual size and stay centered. A prior fix
  made `gum` render at all; everything visible after that was still laid out
  against wrong numbers. Sizing had silently fallen back to an 80x24 canvas because
  `$COLUMNS`/`$LINES` are never exported to a child process and `tput` can't read a
  piped terminal; a new `terminal_size()` helper reads the real pane size directly.
  Menu items were also centered against stale hardcoded widths, off by up to 10
  columns, and the banner assumed a fixed 14-row height; both now measure actual
  content.
- CJK and other wide characters in branch names no longer throw off menu
  centering, since width is now measured per display column instead of per byte.
- Editing failed silently in two places: the SSH picker's `[manual]` record and the
  project picker's editor. Both piped `gum`'s stderr, where `gum` draws its prompt,
  so `ctrl+i` cleared the screen and drew four prompts nowhere. Both now inherit
  stderr like every other `gum` call site, and a regression test pins the stream
  contract.
- SSH sessions now pass the real config file into session-history stamping, so the
  configured half of the SSH registry no longer gets dropped after a session.

Known limitation, not fixed in this release: `gum filter` strips the indent on its
cursor row only, so the highlighted row in the New Worktree branch list jumps to
column 0.

## [0.1.1] - 2026-07-22

_tracks tag `v0.1.1`_

### Fixed
- New agent tabs now open in the current workspace instead of the plugin's install
  directory. `alt+c` runs the "New agent" popup as a Herdr plugin pane, which Herdr
  launches with the plugin's own install dir as cwd; since that directory is itself a
  git checkout, the script's project-root detection resolved to the plugin instead of
  the workspace you invoked it from. The popup now adopts the invoking pane's cwd
  before doing worktree discovery, falling back to the previous behaviour if that
  pane or its cwd is unavailable.

## [0.1.0] - 2026-07-22

_tracks tag `v0.1.0`_

First tagged release of `q.workbench`, a Herdr plugin shipping terminal-multiplexer
actions: launching AI agents in structured tab layouts, fzf pickers for projects and
SSH targets, and restarting agents in place.

### Added
- `config.example.zsh` at the repo root, documenting every configurable setting with
  its default, fully commented out — copy it to write a local config.
- `Q_CODEX_EXTRA_ARGS`, a pass-through slot for extra codex flags, mirroring the
  existing `Q_CLAUDE_EXTRA_ARGS`.

### Changed
- **Breaking:** `Q_UNSAFE_CODEX=1` is gone. To bypass approvals and sandboxing, set
  `Q_CODEX_EXTRA_ARGS='--dangerously-bypass-approvals-and-sandbox'` instead. The
  bypass remains opt-in and is never added automatically.
- **Breaking:** SSH registry settings are renamed from `ZSSH_*` to `Q_SSH_*`
  (`ZSSH_REGISTRY_FILE` → `Q_SSH_REGISTRY_FILE`, `ZSSH_CONFIG_FILE` →
  `Q_SSH_CONFIG_FILE`, `ZSSH_HISTORY_FILE` → `Q_SSH_HISTORY_FILE`).
- Every script that reads a setting now sources `config.zsh`, which owns all
  defaults; scripts read `$Q_FOO` directly instead of each repeating its own
  fallback.

### Fixed
- Project- and SSH-registry settings (`Q_PROJECTS_ROOT`, `Q_PROJECT_REGISTRY_FILE`,
  `Q_SSH_*`) now actually take effect when set in a user config file. Previously
  they were documented as configurable but silently ignored, because the scripts
  that read them never sourced `config.zsh`.

### Note
- Overriding the claude model menu (`Q_AGENT_MODELS` / `Q_AGENT_MODEL_ARGS`) from a
  user config requires declaring them with
  `typeset -gA Q_AGENT_MODELS Q_AGENT_MODEL_ARGS` before assigning — zsh silently
  empties a plain array when converting it to associative, which made such
  overrides fail invisibly.
