# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
