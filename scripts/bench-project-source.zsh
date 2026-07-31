#!/usr/bin/env zsh
# Measure `workbench project source`, the one subcommand fzf reloads on every
# keystroke. Reports the median of 50 warm invocations against the real registry.
#
# The harness spends nothing per sample beyond one fork+exec: EPOCHREALTIME is a zsh
# builtin, so no `date`, `python` or `time` process inflates the number. That floor is
# reported too — a wrapper that spawns a helper per sample adds 2-3 ms of its own and
# makes the result unreadable.

set -eu

cd "${0:A:h:h}"
cargo build --release >/dev/null

zmodload zsh/datetime

bench() {
  local label="$1"
  shift
  local -a samples
  local i start end
  for i in {1..10}; do "$@" >/dev/null 2>&1 || true; done
  for i in {1..50}; do
    start=$EPOCHREALTIME
    "$@" >/dev/null 2>&1 || true
    end=$EPOCHREALTIME
    samples+=$(printf "%.3f" $(( (end - start) * 1000 )))
  done
  printf "%-34s " "$label"
  print -l $samples | sort -n |
    awk '{s[NR]=$1} END {printf "median %6.2f ms   min %6.2f   max %6.2f\n", s[int((NR+1)/2)], s[1], s[NR]}'
}

binary="$PWD/target/release/workbench"

bench "fork+exec floor (/usr/bin/true)" /usr/bin/true
bench "rust: project source" "$binary" project source
bench "zsh:  project-picker-source" zsh scripts/project-picker-source.zsh

# With a query of two or more characters both versions shell out to `zoxide query`,
# which costs about 8 ms on its own. It dominates the comparison, so it is reported
# separately rather than mixed into the number above.
bench "rust: project source <query>" "$binary" project source herd
bench "zsh:  project-picker-source <q>" zsh scripts/project-picker-source.zsh herd
