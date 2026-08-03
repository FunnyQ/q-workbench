# tab-layout-selector — Task System

## Purpose

Each task file is a **self-contained, independently pickable unit**. An executor needs only:

1. The `_context/` files listed in the task's `Required reading` header
2. The task file itself

They should not need to open `PLAN.md` or any other task file. `PLAN.md` is the master spec; `_context/` is its surgical extract; task files describe **what to do** without re-explaining **why**.

## Directory layout

```
tasks/
├── README.md                  ← this file
├── _context/
│   ├── shared.md              ← stack, code style, socket + popup constraints, frozen decisions
│   ├── architecture.md        ← current signatures of every file this plan touches
│   └── rubric.md              ← scoring scale and the shared pass line
└── work/                      ← the whole plan; one linear feature, no parallel tracks
    └── NN-<slug>.md
```

## Reading order for executors

1. `_context/shared.md` — required for every task.
2. Topic-specific `_context/*.md` per the task's `Required reading` header.
3. The task file itself.

## Naming convention

`<bucket>/NN-<kebab-slug>.md` — `NN` is two-digit zero-padded.

## Where to start

`work/01-menu-module.md`. It and `work/02-layout-menu-labels.md` are the two foundation
tasks and touch different files, so they can run in the same wave. Everything after them is
a chain.

Run `cargo test` and `cargo clippy -- -D warnings` before starting, so a pre-existing
failure is not mistaken for one this plan introduced.

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
| work | 01 | Extract the gum menu primitives into `src/flows/menu.rs` | todo | > 4 | — |
| work | 02 | Give tab layouts a menu label | todo | > 4 | — |
| work | 03 | Split the agent popup into a reusable half | todo | > 4 | work/01 |
| work | 04 | The layout menu flow in `src/flows/tab.rs` | todo | > 4 | work/01, work/02, work/03 |
| work | 05 | Route `tab new` and register the Herdr action | todo | > 4 | work/04 |
| work | 06 | Document the action and the two new layout keys | todo | > 4 | work/02, work/05 |
| work | 07 | Final review, rebuild, and manual run | todo | > 4 | work/01, work/02, work/03, work/04, work/05, work/06 |

## Dependency graph

```
work/01
├─→ work/03
├─→ work/04 *
│   └─→ work/05
└─→ work/07 *
work/02
└─→ work/06 *
```

`*` = task has additional dependencies beyond the parent shown above; see the **Task index** for the full `Depends on` list.
<!-- flightplan:generated:end -->

## Known gaps

1. **The new pane's Nerd Font glyph is unchosen** (scope: `work/05-cli-and-manifest.md`)
   Every `[[panes]]` title in `herdr-plugin.toml` starts with a glyph and two spaces. The
   plan fixes the two-space convention but not which glyph. The executor picks one that
   matches the visual weight of the neighbouring entries; Q can swap it later without
   touching code.

2. **The manual Herdr run needs a running Herdr** (scope: `work/07-final-review.md`)
   Five acceptance criteria are a hand-driven run of the action. Without Herdr the closing
   task must be marked `blocked`, not `done`. The automated half of the same claim — that
   the chosen layout is the one built — is a unit test in the tab flow, so the plan stays
   verifiable, but the closing gate does not pass until someone runs it for real.
