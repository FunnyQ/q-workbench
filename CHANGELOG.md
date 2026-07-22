# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
