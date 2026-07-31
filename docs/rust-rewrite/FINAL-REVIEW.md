# Final review — Rust rewrite of `q.workbench`

Reviewed at `b9f528c` plus the fixes recorded here. Herdr 0.7.5, protocol 17, macOS
arm64 (Darwin 25.5.0), Rust release profile.

**Verdict: pass.** No clause carries a `regressed` verdict after the fixes below. Two
correctness defects were found and repaired during this review; both are recorded
against the clause they broke.

One operational step is left for Q and is not a code defect: see
[Cuts and known gaps](#cuts-and-known-gaps), item K-1.

---

## 1. Actions

Six actions in `herdr-plugin.toml`, walked against the installed plugin
(`./bin/workbench`, rebuilt from this tree) and the live Herdr socket.

| Action | Result | Observed |
|---|---|---|
| `new-agent` | pass | `[[panes]] agent` runs `./bin/workbench agent popup`. Menu → tab flow is pinned end to end by `popup_reproduces_the_exact_ten_call_sequence`, which asserts all ten calls in order with every parameter; layout ratios, `Files`/`term` labels and `Q_NO_BANNER` placement now come from the single `build_side_panes` helper. Live socket confirms `tab.create` returns `workspace`/`tab`/`root_pane` in the shape the flow reads. |
| `new-worktree-agent` | pass | Same pane entry plus `--worktree`. Branch selection, `<repo parent>/<repo name>-wt/<branch>` naming and the fallback-to-no-worktree path are covered against a real throwaway git repository (`RepoFixture`), including `slash_in_branch_becomes_dash_in_worktree_directory` and `cancelling_in_a_real_repository_creates_no_worktree_and_no_branch`. The chosen worktree drives `cwd` for all three panes. |
| `project` | pass | `project source` run live against Q's real 60-entry registry emits correct three-line NUL records (sample below). Existing workspace is focused only; new workspace branches on the key — `existing_project_workspace_is_only_focused`, `new_project_enter_creates_injects_then_focuses`, `new_project_alt_enter_creates_plain_workspace_then_focuses`. |
| `ssh` | pass **after fix** | `ssh list`/`ssh sync`/`ssh get` run live against Q's real SSH config and registry. The tab close on disconnect was **broken** and is fixed — see SSH-3 below and finding F-1. |
| `restart-agent` | pass | Kill guard, TERM → 50×100 ms → KILL → 300 ms settle, and the literal unquoted TTY-reset prefix are pinned by `restart_command_keeps_reset_unquoted_and_quotes_launcher_arguments` and `the_detached_worker_survives_a_sigterm_to_its_spawner_group`. Re-injection carries no tab id, the current label as fixed usage, no worktree and `no_layout`, so the side panes survive. |
| `dashboard` | pass | `[[actions]] dashboard` runs `./bin/workbench dashboard` directly (no popup hop). Workspace resolved by label every run, tab created focused, prompt submitted as one argument — `creates_focused_dashboard_tab_with_submitted_prompt`, `submitted_command_round_trips_to_four_arguments`. |

Live `project source` sample (Q's real registry, override cleared):

```
󰉋  cc-plugins
   ~/Projects/q-lab/cc-plugins
   claude · codex · filesystem	/Users/funnyq/Projects/q-lab/cc-plugins
```

### Seams

- **Picker → launcher.** `project pick` builds its inject command through
  `agent::InjectOptions`, not a second string path. `session_command` was building the
  SSH pane command by hand; it now calls `shell::build_command`, so there is exactly
  one command builder in the binary (finding F-6).
- **Restart → pane label.** The worker reuses the current pane label as fixed usage;
  `worker_argv_carries_the_hidden_subcommand_and_the_pane_flag` pins the argv.
- **Picker bindings.** Both pickers wire `execute(...)+reload(...)` back into the same
  binary, quoting the executable path (`bindings_quote_an_executable_path_containing_spaces`).
  All three editor entry points `clear` before drawing.
- **Two entry points, one decision module.** `flows::agent::choose_agent_with_last` is
  the only place a harness/model/usage decision is made. The "is a stored last-choice
  still offerable" rule was implemented twice and is now `state::last_choice_is_valid`,
  called by both. The harness glyph labels were declared twice and now live once.

### Interactive checks not performed by this review

Gum and fzf need a controlling TTY, and driving them would have created tabs and
launched agents in Q's live session. Menu centering geometry, fzf's redraw after an
editor binding and TTY recovery after killing codex are therefore covered by their
tests and by code inspection, not by a human at the terminal. Recorded as K-2.

---

## 2. Parity walk

One row per id in the parity contract's clause index. The index has **64** rows; this
table has **64** rows.

| id | Verdict | Note |
|---|---|---|
| DEV-1 | deviation | Sanctioned. In-pane launcher adopts the popup's harness → model → usage order so both entry points share one decision module. |
| DEV-2 | deviation | Sanctioned. TOML string arrays replace zsh word-splitting; `file_extra_args_preserve_spaces_inside_one_argument` pins it. |
| DEV-3 | deviation | Sanctioned. Every fatal path reports a concrete cause on its subcommand's fixed channel; `every_subcommand_selects_its_fixed_channel`. |
| DEV-4 | deviation | Sanctioned. Protocol guard silent on match, exactly one notification on mismatch; `protocol_mismatch_sends_exactly_one_notification_with_both_numbers`. |
| DEV-5 | deviation | Sanctioned. Restart offers "use last combination" first; `use_last_is_first_and_skips_model_and_usage_menus`. |
| DEV-6 | deviation | Sanctioned. Worktree realised after every menu; `a_completed_choice_still_creates_nothing_until_the_caller_realises_it`. |
| GLY-1 | holds | All 19 glyph literals and their two-space spacing checked codepoint by codepoint against the table; script output in Searches. |
| GLY-2 | holds | `MODEL_TITLE` is `U+F09D1` + two spaces; the comment at `flows/agent.rs` records that the zsh launcher used one. |
| GLY-3 | holds | `strip_pad` removes the leading pad and keeps the glyph; `labels_keep_glyphs_and_append_branches`. |
| MSG-1 | holds | Four notification titles and bodies at `bottom-right`; `non_agent_notifying_subcommands_carry_their_contract_titles`. |
| MSG-2 | holds | Popup cleanup notification passes `sound: "none"`; `notify_sends_the_expected_notification`. |
| MSG-3 | holds | `stderr_preserves_contract_messages_and_prefixes_unnamed_failures`, `unnamed_failure_uses_subcommand_path_on_real_stderr`. |
| MSG-4 | holds | `update_uses_real_stdout_and_success_exit`, `update_prints_the_success_line_to_stdout`. |
| MSG-5 | holds | `<subcommand path>: <chained cause>`, exit 1; verified live (`ssh list: config file … is zsh, not TOML`, exit 1). |
| MSG-6 | holds | Verified live: a failing `project source` produced zero bytes on both channels. |
| RST-1 | holds | Banner, labels and four colour flags pinned by the restart confirm tests. |
| RST-2 | holds | `target_is_focused_agent_or_first_agent_in_tab`. |
| RST-3 | holds | `focus_walk_uses_nested_neighbor_and_directional_focus` walks `left right up down`. |
| RST-4 | holds | Kill guard requires a present, non-zero pgid distinct from the shell pid. |
| RST-5 | holds | TERM, 50 polls at 100 ms, KILL, 300 ms settle. |
| RST-6 | holds | `TTY_RESET` is a literal unquoted prefix; `restart_command_keeps_reset_unquoted_and_quotes_launcher_arguments`. |
| RST-7 | holds | `worker_on_the_agent_pane_itself_skips_the_focus_walk` plus the argv test cover no tab id, current label as fixed usage, no worktree, no layout. |
| RST-8 | holds | `flows/agent.rs` ends in `CommandExt::exec()`; no wrapper process survives. |
| CFG-1 | holds | `missing_file_resolves_every_documented_default` asserts all 11 defaults. |
| CFG-2 | holds | `file_overrides_environment_including_with_empty_values`, `environment_overrides_built_in_defaults_and_splits_extra_args`. |
| CFG-3 | holds | Model tables including per-label extra flags; `launch_commands_match_every_harness_and_model_rule`. |
| CFG-4 | holds | `extra_args_never_gain_implicit_bypass_flags`, `bypass_flags_are_absent_unless_configured`. |
| LAU-1 | holds | Per-harness assembly including `CCR` → `ccr code`. |
| TAB-1 | holds | `popup_reproduces_the_exact_ten_call_sequence` — still green after the side-pane helper extraction, which is what proves the extraction changed nothing. |
| TAB-2 | holds | `Q_NO_BANNER` on the tab and the first split only; now expressed once, in `build_side_panes`. |
| TAB-3 | holds | `popup_cancelled_choice_makes_zero_calls`. |
| TAB-4 | holds | `popup_failure_at_every_post_create_step_closes_and_returns_metadata`; the close uses `{"tab_id": …}` and always did on this path. |
| TAB-5 | holds | `launcher_builds_the_required_layout_sequence`, `no_layout_skips_splits_and_tab_rename_is_optional`. |
| WRK-1 | holds | Prune, exclude checked-out branches, auto-name on empty; `a_checked_out_branch_is_not_offered`, `an_empty_branch_name_becomes_a_timestamped_one`. |
| WRK-2 | holds | `slash_in_branch_becomes_dash_in_worktree_directory`. |
| WRK-3 | holds | `failed_worktree_choice_normalises_to_the_no_worktree_choice`. |
| WRK-4 | holds | The chosen `project_dir` is the `cwd` passed to the tab and to both splits. |
| LBL-1 | holds | `labels_keep_glyphs_and_append_branches`, `a_usage_label_ending_in_the_branch_name_survives_normalisation`. |
| SIZ-1 | holds | `pane_viewport` reads `pane.layout`, then `COLUMNS`/`LINES`, then `tput`. |
| CWD-1 | holds | `popup_cwd_prefers_plugin_context_and_falls_back_to_active_pane`. The popup and the picker deliberately differ on `foreground_cwd`; that difference is now named by `flows::PaneCwd` instead of being two silently divergent copies (finding F-8). |
| RGP-1 | holds | `atomic_write_is_pretty_printed_and_has_a_trailing_newline`, `serialization_failure_keeps_the_registry_and_removes_the_temporary_file`. |
| RGP-2 | holds | Seven `canonical_project` tests including the projects-root exception. |
| RGP-3 | holds | `test_discover_claude`, `test_discover_codex`, `test_discover_filesystem_prunes_before_descending`. The Claude walk is now one pass instead of two (finding F-10); `test_discover_claude` and `test_discover_claude_skips_an_unreadable_transcript` pin the output. |
| RGP-4 | holds | `merge_preserves_manual_and_sorts_unique_sources`, `merge_clears_sources_when_a_project_disappears`. The `canonical_project` memo (F-9) is keyed on the raw candidate and does not change accumulation order. |
| RGP-5 | holds | `operation_guards_use_the_contract_messages`, `candidate_rows_cover_scan_and_every_rescan_marker`, four `edit_*` tests. |
| RGP-6 | holds | `cancelled_and_empty_scan_leave_registry_byte_for_byte_unchanged`, `edit_cancellation_preserves_registry_and_empty_name_uses_basename`. |
| RGS-1 | holds | `registry_json_is_byte_identical_to_zsh_output`. |
| RGS-2 | holds | `reconciliation_drops_stale_config_and_preserves_manual_targets`. This is the clause finding F-2 was violating from the session path. |
| RGS-3 | holds | `sync_seeds_only_absent_or_invalid_registries`. |
| RGS-4 | holds | `remove_hides_config_and_deletes_manual_targets`. |
| RGS-5 | holds | `use_resolves_alias_and_collapses_a_unique_manual_target`, `use_keeps_manual_target_when_two_config_targets_match`. |
| RGS-6 | holds | `list_is_nul_delimited_and_uses_the_contract_sort_and_layout`; verified live against Q's real registry. |
| PIK-1 | holds | `project_fzf_arguments_match_the_parity_contract` covers flags, prompt, border label and all three bindings. |
| PIK-2 | holds | `emits_an_unregistered_zoxide_directory_after_registry_rows`, `suppresses_ineligible_zoxide_fallbacks`, `a_missing_zoxide_binary_is_not_an_error`. |
| PIK-3 | holds | Three `*_project_*workspace*` tests cover focus-only and both keys. |
| PIK-4 | holds | `emits_exact_three_line_nul_delimited_records_in_picker_order`, `home_is_collapsed_only_in_the_display_path`; verified live. |
| PIK-5 | holds | `ssh_fzf_arguments_match_the_parity_contract`. |
| PIK-6 | holds | All three binding-invoked editors run `Command::new("clear")` before drawing. |
| SSH-1 | holds | `validates_every_ssh_config_field`, `missing_and_empty_targets_fail_before_ui`, duplicate-alias refusal. |
| SSH-2 | holds | `renders_exact_block_bytes_for_all_file_and_user_combinations`; the mode-preserving atomic write is deliberately its own implementation. |
| SSH-3 | holds **(was regressed; fixed)** | `tab.close` was sent as `{"id": tab_id}`, which protocol 17 rejects — see finding F-1. Every SSH tab therefore stayed open after disconnect, and the error was swallowed by design. Fixed and confirmed against live Herdr 0.7.5. |
| SSH-4 | holds **(was regressed; fixed)** | The clean-exit stamp passed `/dev/null` as the SSH config, so `use_target`'s reconciling `sync` deleted every `source: "config"` entry — see finding F-2. Fixed; `zero_exit_closes_tab_and_stamps_registry` now asserts the config-sourced entry survives. |
| DSH-1 | holds | `missing_workspace_preserves_message_and_does_not_create_a_tab`, `workspace_list_failure_has_exact_reporting_metadata`. |
| DSH-2 | holds | `creates_focused_dashboard_tab_with_submitted_prompt`, `submitted_command_round_trips_to_four_arguments`. |

Counts: 64 rows, 64 ids. 58 `holds`, 6 `deviation`, 0 `regressed`.

---

## 3. Measurements

Release profile, Rust 1.97.1, `scripts/bench-project-source.zsh` (restored — see K-3):
median of 50 warm invocations, timed with zsh's `EPOCHREALTIME` builtin so the harness
spawns nothing per sample.

| Figure | Measured | Plan's prediction | Verdict |
|---|---|---|---|
| Release binary size | **1.9 MB** (`target/release`), 1.8 MB installed at `bin/workbench` | "Estimated 2–3 MB, unmeasured" (PLAN.md:384) | Under the low end |
| Per-invocation startup (`--version`, full clap path) | **3.41 ms** | "Rust startup ≈ 3.6 ms" (PLAN.md:216) | Matches |
| fork+exec floor (`/usr/bin/true`) | **2.43 ms** | — | The floor every process pays |
| `project source`, no query | **3.35 ms** | ≤ 5 ms budget; zsh measured **14.6 ms** | 4.4× faster, 1.65 ms under budget |
| `project source <query>` | **11.24 ms** | zsh measured 31.8 ms | 2.8× faster; `zoxide query` is ~8 ms of it and is required for parity |
| Socket vs CLI | `herdr ping` over the socket returns in the 3.4 ms startup total, against ~9.8 ms for a `herdr` CLI spawn (PLAN.md:215) | "~4× faster" | Holds |

So the subcommand costs about 0.9 ms beyond the fork+exec every process pays. The
per-keystroke path — the rewrite's stated main motivation — is comfortably inside its
budget with the whole registry loaded.

Fast-path measurement (altitude lens asked whether `project_source_fast_path` earns its
keep). Same binary, the fast path disabled by an environment flag, two rounds:

| Path | Median |
|---|---|
| argv fast path | 3.52 / 3.56 ms |
| full clap path | 3.64 / 3.78 ms |

Clap's parser construction costs **0.15–0.22 ms**, about 4–6% of the hot path and 4% of
the budget. Kept — reasoning in F-13.

---

## 4. Searches

```
$ rg -n '"herdr"|\bjq\b' src/
src/main.rs:761:            vec!["workbench", "herdr", "ping"],
src/main.rs:898:                vec!["workbench", "herdr", "ping"],
src/main.rs:1143:        let cli = Cli::try_parse_from(["workbench", "herdr", "ping"]).unwrap();
src/main.rs:1193:            vec!["workbench", "herdr", "ping"],
src/registry/project.rs:447:    // numeric offsets, while jq's contract is exactly `%Y-%m-%dT%H:%M:%SZ`.
src/registry/project.rs:520:/// the same order jq's `keys[]` produces, so the two versions list identically.
src/registry/project.rs:944:        // serde_json omits the final newline, but jq writes one. Add it explicitly
src/registry/project.rs:1215:        // edited name, and rows come out in path order — both as jq produces them.
```

Nothing here spawns either binary. The four `main.rs` hits are argv literals in tests
for **this binary's own** `workbench herdr ping` subcommand; the four `project.rs` hits
are comments explaining why a format matches what jq used to produce.

```
$ rg -c '"herdr", "plugin", "pane", "open"' herdr-plugin.toml
5
```

The five scoped-exempt actions (`new-agent`, `new-worktree-agent`, `project`, `ssh`,
`restart-agent`) are present and correct: each names `--plugin q.workbench`, its own
`--entrypoint`, and `--placement popup` with the geometry the parity contract records.
The sixth action, `dashboard`, invokes `./bin/workbench` directly. All five `[[panes]]`
commands are `./bin/workbench …`.

```
$ rg -n 'unimplemented' src/
(no matches)
```

Every subcommand in the shared context's surface exists and does real work. All six
top-level groups are present in `--help`; `project`, `ssh`, `config` and `herdr` were
run live against Q's real registries and the live socket, and `agent` and `dashboard`
were not (they create tabs and launch agents in Q's session — K-2). Every
`ProjectCommand`, `SshCommand` and `ConfigCommand` leaf is exercised by a test, and
`every_leaf_parses_with_all_supported_arguments` covers the whole argument surface.

Glyph audit (GLY-1), all 19 literals checked against the codepoint table:

```
U+F169F  Launch Agent  ok      U+F09D1  main                      ok
U+F169F  opencode      ok      U+0F27B  Usage                     ok
U+F169F  agent         ok      U+0F442  discuss                   ok
U+F15CE  claude code   ok      U+0F4AF  review                    ok
U+0EE0D  codex         ok      U+0EAD8  debug                     ok
U+F09D1  claude code   ok      U+F19B9  let me write…             ok
U+F0968  Files         ok      U+0F489  term                      ok
U+F024B  (row prefix)  ok      U+F08A9  (ssh tab label)           ok
U+0EACD  Dashboard Launcher ok U+F002A  Current session will end  ok
U+F0709  use last:     ok
```

Build gates:

```
$ cargo test          → 190 passed, 0 failed  (158 lib + 16 bin + 16 integration)
$ cargo clippy --all-targets -- -D warnings   → clean
```

Revertibility, checked in a scratch worktree at HEAD:

```
$ git revert --no-commit --no-edit 7731a62    → clean, no conflicts
  restores 14 zsh scripts, config.example.zsh, dev/, the zsh manifest
$ for t in tests/*.test.zsh; do zsh "$t"; done → all pass
```

---

## 5. Cuts and known gaps

### Findings fixed in this review

| id | Lens | What was wrong | Fix |
|---|---|---|---|
| F-1 | cross-vendor (codex) | `flows/ssh.rs` sent `tab.close` as `{"id": tab_id}`. Protocol 17 requires `tab_id`; the request was rejected and the error swallowed by design, so **every SSH tab stayed open after disconnect** (SSH-3). | Send `{"tab_id": …}`. Confirmed against live Herdr 0.7.5: `{"id"}` returns `missing field tab_id`, `{"tab_id"}` returns `ok`. Both call-site tests updated. |
| F-2 | cross-vendor (codex) | The clean-exit stamp called `use_target(registry, /dev/null, history, target)`. `use_target` opens with a reconciling `sync`, and an empty config means **every `source: "config"` target was deleted from the registry** on each successful SSH session (SSH-4, RGS-2). | Thread the real `ssh_config_file` through `session`. `zero_exit_closes_tab_and_stamps_registry` now writes a real config, and asserts the config-sourced entry and its hostname survive the stamp. |
| F-3 | integration (found in this review) | On a machine that ran the zsh version, `Q_WORKBENCH_LOCAL_CONFIG` is still exported pointing at `config.zsh`. It overrides the resolved TOML path, so **every subcommand died** with a TOML syntax error on `typeset -gA`. Reproduced on Q's own machine. | `Config::load` refuses a `.zsh` path with a message naming `workbench config migrate --write` and the stale variable. New test; README section added. Still needs Q to act — K-1. |
| F-4 | simplification | `state.rs` shelled out to `mktemp` for every agent-state write, a third atomic-write implementation. | Reuse `registry::project::write_json_atomically`; the subprocess is gone. |
| F-5 | reuse, altitude, simplification | `HARNESS_CLAUDE/CODEX/OPENCODE` declared twice with identical glyphs, and the "is a stored choice still valid" rule implemented twice. A glyph edit in one file would silently invalidate every stored record. | Constants live once in `state.rs`; `state::last_choice_is_valid` is the single rule, used by `get_for_pane` and the harness menu. |
| F-6 | reuse | `picker::session_command` re-implemented `shell::build_command`. | Calls it. One command builder in the binary. |
| F-7 | reuse, altitude | `apply_launch_layout` and `build_popup_tab` duplicated the whole six-call side-pane sequence line for line. | Extracted `build_side_panes`. `popup_reproduces_the_exact_ten_call_sequence` still green, which is the proof nothing moved. The launch path also gained the popup's empty-pane-id guards. |
| F-8 | reuse, simplification | Two "adopt invoking pane cwd" helpers that had already drifted (`foreground_cwd.or(cwd)` vs `cwd`). | One `flows::invoking_pane_cwd` taking a `PaneCwd` enum. The difference is preserved and now documented as intentional rather than looking like drift. |
| F-9 | efficiency | `discovered_projects` ran `canonical_project` — a `git rev-parse` plus a `canonicalize` — once per raw candidate. Hundreds of transcripts map to a handful of repositories. The zsh original piped candidates through `sort -u` first, so this was a regression. | Memoised on the raw candidate path. |
| F-10 | efficiency | `discover_claude_projects` walked `~/.claude/projects` recursively twice. | One walk, partitioned. |
| F-11 | efficiency | `ssh::edit_with` called `sync` and then `use_target`, which opens with the same `sync` — two full `ssh -G` sweeps and two registry writes per edit. | Dropped the explicit call. |
| F-12 | efficiency | `state::get_for_pane` called `Config::load()` on every restart although the caller already held a loaded `&Config`. | Takes `&Config`. |
| F-13 | simplification, altitude, reuse | Dead code and dead paths: unreachable second `SocketClient::new()`; `notify_and_exit` with no callers; a self-skipping test for a zsh script deleted at cutover; `git rev-parse --show-toplevel` implemented twice; the crate's only clippy warning. | All removed or shared. `git_toplevel` is now one function; the stricter empty-output handling wins. |
| F-14 | simplification, altitude | Every module compiled three times: `main.rs` re-declared all seven under blanket `#[allow(dead_code)]`, `lib.rs` built them again for nobody, and `tests/socket_client.rs` `#[path]`-included an eighth copy. | `main.rs` and the test now consume the lib. All six blanket `#[allow(dead_code)]` attributes are gone, and five duplicated unit tests stopped running twice. |

### Findings considered and not fixed

| id | Lens | Finding | Why not |
|---|---|---|---|
| K-4 | altitude | Delete `project_source_fast_path`; clap's tree is "a rounding error". | Measured, not assumed: it is 0.15–0.22 ms of a 3.5 ms hot path. It is not a duplicated parser — it hands every dash, non-UTF-8 or extra argument straight back to clap rather than re-implementing clap's rules, and the per-keystroke path is the rewrite's stated motivation. Kept, with the measurement recorded above. |
| K-5 | simplification | Delete the `pane_current` and `pane_focus` trait methods. | `foundation/02` names them in the helper roster. They are `pub` on a lib target, cost nothing at runtime, and removing them is a spec deviation for no gain. |
| K-6 | simplification | Drop the unread response fields in `herdr/types.rs`. | They document the protocol shape next to the code that decodes it, which is worth more than the lines they cost. Serde ignores unknown fields either way, so nothing breaks if Herdr adds more. |
| K-7 | reuse | One shared epoch-seconds helper for four `SystemTime::now().duration_since(UNIX_EPOCH)` call sites. | A new module (or a cross-module import) for a four-line stdlib call costs more than the repetition. There is no drift hazard: the value is defined by the epoch, not by us. |
| K-8 | altitude | `main.rs` `report_stderr` matches six hardcoded `ssh edit` message prefixes instead of a typed marker. | Real drift hazard, agreed, but converting it means touching every `ssh edit` contract error and the MSG-3 tests at final-review time. Deferred as a follow-up, not a blocker: `stderr_preserves_contract_messages_and_prefixes_unnamed_failures` fails loudly if a message changes. |
| K-9 | simplification | Drop `#[allow(clippy::needless_return)]` by removing `return` from every arm of `Cli::run`. | ~200 lines of pure churn with no behavioural or readability gain worth that diff at this stage. |
| K-10 | altitude, simplification | The `0..1000` `create_new` retry loop in `write_json_atomically`; `main.rs` migration write is a fourth temp+rename. | The loop is tested and correct, and the migration writer emits raw bytes rather than JSON. Consolidating them is a refactor of the one path that must never corrupt a registry, for no measurable gain. |
| K-11 | efficiency | `wait_for_signal_pipe` polls at 50 ms for the life of every SSH session; use SIGCHLD instead. | Correct in principle. Changing signal handling in the flow that owns a live connection, at final review, is not a safe trade; the wakeups are cheap and the timeout is also what catches a child that exits without writing to the pipe. |
| K-12 | efficiency | Carry `repo_root` on `AgentChoice` to avoid a second `git rev-parse` per launch. | One subprocess per action, not per keystroke. It needs a field on `AgentChoice` and touches its test literals. Deferred. |
| K-13 | altitude | Merge `FileConfig` and `PartialConfig` into one struct deriving both traits. | `deny_unknown_fields` and `skip_serializing_if` can coexist, but the migration round-trip is pinned byte for byte by four tests. Not worth risking the one path a user runs exactly once. |

### Cut polish tasks

None. `docs/rust-rewrite/tasks/polish/` holds six tasks (`01`–`06`) and all six carry
`Status: done`. Nothing was cut.

### Open gaps

- **K-1 — Q must finish the config migration.** This is the one thing standing between
  the committed binary and a working plugin on Q's machine, and it is Q's data, so this
  review did not do it. Two steps: run `./bin/workbench config migrate --write`, then
  unset `Q_WORKBENCH_LOCAL_CONFIG` (or repoint it at the new `config.toml`) in the shell
  setup that exports it — it lives outside this repo. Until then the binary refuses to
  run and says so.
- **K-2 — three behaviours have no automated coverage and were not driven by hand:**
  gum menu centering on a real TTY, fzf's redraw after an editor binding, and TTY
  recovery after killing codex. Their inputs are pinned by tests
  (`the_centering_geometry_matches_the_popup`, the `clear` calls, `TTY_RESET`), but the
  visible result was not observed. Worth one manual pass at the terminal.
- **K-3 — `scripts/bench-project-source.zsh` was deleted at cutover** along with the 14
  implementation scripts, taking the measurement harness with it and leaving a dangling
  reference in `flows/picker.rs`. Restored, minus the zsh comparison arms, which cannot
  run now. `git show 7731a62^:scripts/project-picker-source.zsh` brings the old script
  back if the comparison is ever needed again.

---

## Did it meet the goal

> Replace the 14 zsh implementation scripts in the `q.workbench` Herdr plugin with a
> single committed Rust binary that talks to Herdr's Unix-socket API directly, at
> behavioural parity plus six sanctioned deviations, cutting over in one revertible
> commit.

- **One binary.** All 14 zsh implementation scripts are gone; `bin/workbench` is
  committed and every `[[panes]]` command and the `dashboard` action invoke it.
- **Socket, not CLI.** No `herdr` spawn anywhere in `src/`. The five `[[actions]]` that
  invoke `herdr plugin pane open` are the scoped exemption, still present and correct.
- **No jq.** No `jq` invocation anywhere; the four matches are comments.
- **Parity plus six deviations.** 64 clause ids walked, 58 hold, 6 are the sanctioned
  deviations, none regressed.
- **The duplication is gone.** One decision module, one command builder, one defaults
  source, one atomic JSON writer, one glyph table, one `git_toplevel`, one plugin-context
  reader, one side-pane builder.
- **Faster.** The per-keystroke path is 3.35 ms against zsh's 14.6 ms, inside a 5 ms
  budget, in a 1.9 MB binary that starts in 3.41 ms.
- **Revertible.** `git revert 7731a62` applies cleanly and the restored zsh suite passes.
