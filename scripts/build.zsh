#!/usr/bin/env zsh
set -eu
cd "${0:A:h:h}"
cargo build --release
mkdir -p bin
# `cp` writes through the existing inode and macOS keeps the old code signature cached
# against it, so the rebuilt binary is SIGKILLed (exit 137) until it is re-signed. A fresh
# file plus `mv` swaps the inode instead, leaving nothing stale to match.
cp target/release/workbench bin/workbench.new
mv -f bin/workbench.new bin/workbench
print -r -- "built bin/workbench ($(du -h bin/workbench | cut -f1))"
