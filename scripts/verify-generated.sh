#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

generated=(
  src/types/generated.rs
  src/enums/generated.rs
  src/methods/generated.rs
  src/bot/generated.rs
  src/types/bound.rs
)
snapshot="$(mktemp -d)"
trap 'rm -rf "$snapshot"' EXIT
for file in "${generated[@]}"; do
  mkdir -p "$snapshot/$(dirname "$file")"
  cp "$file" "$snapshot/$file"
done

cargo run -p xtask -- generate --upstream aiogram
cargo fmt --all
for file in "${generated[@]}"; do
  if ! cmp -s "$snapshot/$file" "$file"; then
    diff -u "$snapshot/$file" "$file" || true
    echo "generated source drift: $file" >&2
    exit 1
  fi
done
