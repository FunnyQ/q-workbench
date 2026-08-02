# workbench-config-schema — Task System

## Purpose

Each task file is a **self-contained, independently pickable unit**. An executor needs only:

1. The `_context/` files listed in the task's `Required reading` header
2. The task file itself

They should not need to open `PLAN.md` or any other task file. `PLAN.md` is the master spec; `_context/` is its surgical extract; task files describe **what to do** without re-explaining **why**.

## Directory layout

```
tasks/
├── README.md                  ← this file
├── _context/                  ← shared context (every task references these)
│   ├── shared.md              ← decisions, conventions, commit style
│   └── <other>.md             ← topic-specific shared context
└── <bucket>/                  ← bucket description
    └── NN-<slug>.md
```

## Reading order for executors

1. `_context/shared.md` — required for every task.
2. Topic-specific `_context/*.md` per the task's `Required reading` header.
3. The task file itself.

## Naming convention

`<bucket>/NN-<kebab-slug>.md` — `NN` is two-digit zero-padded.

## Where to start

<!-- Edit this with the first task to pick up, e.g. `ui/01-fixture-shell.md`. -->

<!-- flightplan:generated:start -->
## Status conventions

Each task header has a `> **Status**: <status>` line. Executors update it as they go:

- `todo` — not started
- `in-progress` — actively being worked on
- `done` — merged / shipped
- `blocked` — waiting on a decision, upstream task, or external resource

## Task index

| Bucket | NN | Title | Status | Pass line | Depends on |
|---|---|---|---|---|---|
| config | 01 | Remove the zsh migration surface | todo | > 4 | — |
| config | 02 | Schema types for layouts and agents | todo | > 4 | config/01 |
| config | 03 | Built-in defaults reproduce today | todo | > 4 | config/02 |
| config | 04 | Load-time validation | todo | > 4 | config/03 |
| launch | 01 | Menus read the layout | todo | > 4 | config/04, launch/02 |
| launch | 02 | Build launch argv from agents | todo | > 4 | config/04 |
| launch | 03 | Side panes from the layout | todo | > 4 | config/04 |
| wiring | 01 | Layout flag and manifest entries | todo | > 4 | launch/01, launch/03 |
| wiring | 02 | Agent state v2 keyed on stable ids | todo | > 4 | launch/01, wiring/01 |
| wiring | 03 | Example config, changelog, and prose docs | todo | > 4 | launch/02, wiring/01 |
| wiring | 04 | Final review | todo | > 4 | launch/02, wiring/02, wiring/03 |

## Dependency graph

```
config/01
└─→ config/02
    └─→ config/03
        └─→ config/04
            ├─→ launch/01 *
            │   ├─→ wiring/01 *
            │   └─→ wiring/02 *
            ├─→ launch/02
            │   ├─→ wiring/03 *
            │   └─→ wiring/04 *
            └─→ launch/03
```

`*` = task has additional dependencies beyond the parent shown above; see the **Task index** for the full `Depends on` list.

## Cross-bucket dependencies

<!-- Add a third column (Why) by hand if the rationale would help executors. -->

| Task | Depends on |
|---|---|
| launch/01 | config/04 |
| launch/02 | config/04 |
| launch/03 | config/04 |
| wiring/01 | launch/01, launch/03 |
| wiring/02 | launch/01 |
| wiring/03 | launch/02 |
| wiring/04 | launch/02 |
<!-- flightplan:generated:end -->

## Known gaps

<!-- Human-authored. List unresolved decisions or upstream blockers here. -->

- **`agent inject`'s pre-menu pane label is an assumption.** `inject()` renames the pane before any menu has run, so no chosen label exists yet. The CLI wiring task implements it as: the layout's root-pane `label` when the layout pins one, else today's `AGENT_LABEL` constant. Nobody confirmed that is what Q wants. Flag it in the completion report rather than silently keeping it.

- **The compatibility bridge sits inside the schema task, not in a task of its own.** Review raised this twice as mixed concerns — the schema task also rewrites the model menu's option list, the `build_launch` body, and the claude arm of `last_choice_is_valid`, all of which later tasks rewrite again. Kept on purpose. Deleting the five flat fields breaks three call sites, and the bridge is the only thing that lets the schema task's own `cargo test` gate run. It is one method plus three inlined lookups, fully specified, and each later task deletes its own share. Splitting it would move the same twenty lines and add a dependency edge without removing any work. **Do not "fix" this by splitting it.**

- **`ratio` semantics are inferred, not measured.** `herdr = 1 - ratio` reproduces today's `0.38` and `0.9` from config values `0.62` and `0.1`, and matches what Q sees on screen. Herdr itself does not document the field. Worth measuring against a live session while implementing the pane builder, and replacing the inferred rule with a documented one.

- **Workspace creation from a predefined list is deferred entirely.** `[[workspaces]]` was designed for it and has been removed from this plan's scope. `dashboard_workspace` keeps its current meaning — a literal Herdr workspace label. That feature needs its own plan.
