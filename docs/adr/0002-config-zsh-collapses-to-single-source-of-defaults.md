# ADR-0002: config.zsh collapses to the single source of defaults

- Status: Accepted
- Date: 2026-07-22

## Context

Config variables such as `Q_PROJECTS_ROOT` and `ZSSH_CONFIG_FILE` were documented in the README under Configuration as user-settable, implying any script could pick them up. In practice only the three launchers sourced `config.zsh`; the two registries (`project-registry.zsh`, `ssh-target-registry.zsh`) and the pickers (`project-picker-source.zsh` and the ssh picker) never sourced it, so setting those variables in user config had no effect at all. The documentation was wrong, not the user.

Two details of the fix look like things a later cleanup pass would "simplify" away, but both are deliberate:

1. Scripts read the bare `$Q_FOO` variable directly, with no per-script `:-` fallback duplicating the default.
2. `config.zsh` exports the resolved `Q_WORKBENCH_CONFIG_DIR` / `Q_WORKBENCH_LOCAL_CONFIG` paths, which looks like a stray side effect in a file that's supposed to be side-effect free.

## Considered alternatives

- Keep each consuming script defaulting its own config var with a local `:-` fallback. Rejected: this is exactly the drift the change removes — two scripts could end up with different defaults for the same setting if only one fallback is updated later.
- Leave `config.zsh` sourced only by the three launchers, and have the registries/pickers keep reading raw env vars with no default at all. Rejected: this is the status quo bug — the README's Configuration claims stay false for anything that isn't a launcher.
- Keep `config.zsh` free of exports, treating it as a pure function-only file. Considered and rejected: pickers re-source `config.zsh` on every fzf reload, so without the export each reload would re-run a `herdr` shellout to re-resolve the same paths.

## Decision

`config.zsh` is now sourced by every script that reads config, not just the three launchers: both registries and both pickers source it too. Consuming scripts read the bare `$Q_FOO` variable with no local `:-` fallback — the default lives in exactly one place. `config.zsh` also exports the resolved `Q_WORKBENCH_CONFIG_DIR` and `Q_WORKBENCH_LOCAL_CONFIG` paths so that repeated sourcing (once per fzf reload) does not repeat the underlying `herdr` shellout.

## Consequences

- A config default changes in exactly one place and every consumer picks it up consistently; the README's Configuration section is now actually true for `Q_PROJECTS_ROOT`, `ZSSH_CONFIG_FILE`, and similar variables.
- Adding a local `:-` fallback back into a consuming script is a regression, not a safety net — it reintroduces the exact drift this decision removed. A bare `$Q_FOO` read in a consuming script is intentional, not an oversight.
- The `export` lines in `config.zsh` must stay even though the file is otherwise side-effect free: removing them to "purify" the file would add a `herdr` shellout to every fzf reload.

## Evidence

- **README documented config vars that consuming scripts never read** — `Q_PROJECTS_ROOT` and `ZSSH_CONFIG_FILE` were listed under Configuration as user-settable, but the two registries and two pickers never sourced `config.zsh`, so setting them had no effect; fixed by making every config-reading script source `config.zsh` and read its bare variables with no local `:-` fallback, since a second fallback is the drift this removes.
  Session `57f49776-070f-4d5d-a0b4-e9fb6c405740`, entry `6a075947-eb30-4f45-a42b-490b0bd53186`, 2026-07-22.
- **Exported resolved paths are a reload-performance cache, not stray side effects** — pickers re-source `config.zsh` on every fzf reload; without exporting `Q_WORKBENCH_CONFIG_DIR`/`Q_WORKBENCH_LOCAL_CONFIG` once resolved, each reload would re-run a `herdr` shellout to resolve them again.
  Session `57f49776-070f-4d5d-a0b4-e9fb6c405740`, entry `6a075947-eb30-4f45-a42b-490b0bd53186`, 2026-07-22.
