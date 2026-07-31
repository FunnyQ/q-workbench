#!/usr/bin/env zsh
set -eu
cd "${0:A:h:h}"
cargo build --release
mkdir -p bin
cp target/release/workbench bin/workbench
print -r -- "built bin/workbench ($(du -h bin/workbench | cut -f1))"
