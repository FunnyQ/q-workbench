# rust-rewrite — Task System

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
| agent | 01 | Shared agent menu flow | todo | > 4 | foundation/03 |
| agent | 02 | `workbench agent popup` | todo | > 4 | agent/01, foundation/05 |
| agent | 03 | `workbench agent launch` and `agent inject` | todo | > 4 | agent/01, foundation/05 |
| agent | 04 | `workbench agent restart` | todo | > 4 | agent/03 |
| agent | 05 | `workbench dashboard` | todo | > 4 | foundation/03, foundation/05 |
| cutover | 01 | Flip the manifest and delete the zsh implementation | todo | > 4 | foundation/04, foundation/06, polish/01, polish/05, polish/06 |
| cutover | 02 | Documentation and release wiring | todo | > 4 | cutover/01 |
| cutover | 03 | Final review | todo | > 4 | cutover/02 |
| foundation | 01 | Cargo skeleton, clap dispatch, and the build script | todo | > 4 | — |
| foundation | 02 | Herdr socket client | todo | > 4 | foundation/01 |
| foundation | 03 | TOML configuration | todo | > 4 | foundation/01 |
| foundation | 04 | `workbench config migrate` | todo | > 4 | foundation/03 |
| foundation | 05 | Shell quoting and the notification helper | todo | > 4 | foundation/02 |
| foundation | 06 | Dev plugin harness | todo | > 4 | foundation/01 |
| picker | 01 | `workbench project source` | todo | > 4 | registry/02 |
| picker | 02 | `workbench project pick` | todo | > 4 | picker/01, agent/03 |
| picker | 03 | `workbench ssh session` | todo | > 4 | foundation/02, registry/05 |
| picker | 04 | `workbench ssh edit` | todo | > 4 | picker/03, registry/05 |
| picker | 05 | `workbench ssh pick` | todo | > 4 | picker/03, picker/04, foundation/05 |
| polish | 01 | Protocol guard | todo | > 4 | foundation/05 |
| polish | 02 | Error-reporting core and the agent flows | todo | > 4 | agent/02, agent/03, agent/04 |
| polish | 03 | Route the picker, SSH and dashboard flows through the reporting core | todo | > 4 | polish/02, picker/02, picker/05, agent/05 |
| polish | 04 | The stderr channel and the project subcommands | todo | > 4 | polish/03, registry/02, registry/03, picker/01 |
| polish | 05 | Error reporting for the SSH and config subcommands | todo | > 4 | polish/04, foundation/04, registry/05, picker/04 |
| polish | 06 | Remember the last harness and model per pane | todo | > 4 | agent/03, agent/04 |
| registry | 01 | Project discovery and canonicalisation | todo | > 4 | foundation/03 |
| registry | 02 | Project registry storage and non-interactive operations | todo | > 4 | registry/01 |
| registry | 03 | Interactive project review and edit | todo | > 4 | registry/02 |
| registry | 04 | SSH config and history discovery | todo | > 4 | foundation/03 |
| registry | 05 | SSH registry store, reconciliation and operations | todo | > 4 | registry/04 |

## Dependency graph

```
foundation/01
├─→ foundation/02
│   ├─→ foundation/05
│   │   └─→ polish/01
│   └─→ picker/03 *
│       ├─→ picker/04 *
│       └─→ picker/05 *
├─→ foundation/03
│   ├─→ agent/01
│   │   ├─→ agent/02 *
│   │   │   └─→ polish/02 *
│   │   │       └─→ polish/03 *
│   │   │           └─→ polish/04 *
│   │   │               └─→ polish/05 *
│   │   └─→ agent/03 *
│   │       ├─→ agent/04
│   │       └─→ polish/06 *
│   ├─→ agent/05 *
│   ├─→ foundation/04
│   │   └─→ cutover/01 *
│   │       └─→ cutover/02
│   │           └─→ cutover/03
│   ├─→ registry/01
│   │   └─→ registry/02
│   │       ├─→ picker/01
│   │       │   └─→ picker/02 *
│   │       └─→ registry/03
│   └─→ registry/04
│       └─→ registry/05
└─→ foundation/06
```

`*` = task has additional dependencies beyond the parent shown above; see the **Task index** for the full `Depends on` list.

## Cross-bucket dependencies

<!-- Add a third column (Why) by hand if the rationale would help executors. -->

| Task | Depends on |
|---|---|
| picker/05 | foundation/05 |
| picker/04 | registry/05 |
| picker/02 | agent/03 |
| picker/01 | registry/02 |
| picker/03 | foundation/02, registry/05 |
| agent/03 | foundation/05 |
| agent/02 | foundation/05 |
| agent/01 | foundation/03 |
| agent/05 | foundation/03, foundation/05 |
| cutover/01 | foundation/04, foundation/06, polish/01, polish/05, polish/06 |
| polish/05 | foundation/04, registry/05, picker/04 |
| polish/03 | picker/02, picker/05, agent/05 |
| polish/06 | agent/03, agent/04 |
| polish/02 | agent/02, agent/03, agent/04 |
| polish/01 | foundation/05 |
| polish/04 | registry/02, registry/03, picker/01 |
| registry/04 | foundation/03 |
| registry/01 | foundation/03 |
<!-- flightplan:generated:end -->

## Known gaps

<!-- Human-authored. List unresolved decisions or upstream blockers here. -->

Nothing is blocked. Two things are unmeasured, and both are settled early by the tasks
that depend on them:

- **Release binary size** with `clap`, `serde`, `serde_json`, `toml` and `time` linked
  in. Estimated 2–3 MB. `foundation/01` measures it and only tunes the release profile
  if it exceeds 5 MB.
- **Whether `gum` renders identically when spawned from Rust rather than zsh.** TTY
  inheritance is expected to be equivalent. `agent/01` settles it; the fallback is
  passing an explicit `--width`.

Record here anything cut during execution:

- **Cut polish items** — none yet. Deviations 3, 4 and 5 in the parity contract are
  individually cuttable. To cut one, set that task's `Status` to `done`, add a line to
  its Goal saying it was cut and why, and list it here. Do not delete the task file.
