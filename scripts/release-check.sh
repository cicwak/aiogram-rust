#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

requested_version="${1:-}"
python3 - "$requested_version" <<'PY'
import sys
import tomllib
from pathlib import Path

requested = sys.argv[1]
cargo = tomllib.loads(Path("Cargo.toml").read_text())
compatibility = tomllib.loads(Path("compatibility.toml").read_text())
cargo_version = cargo["package"]["version"]
port_version = compatibility["port"]["version"]
if cargo_version != port_version:
    raise SystemExit(f"Cargo version {cargo_version} != compatibility version {port_version}")
if requested and requested != cargo_version:
    raise SystemExit(f"release version {requested} != crate version {cargo_version}")
print(f"release coordinates verified: aiogram-rust {cargo_version}")
PY

scripts/fetch-upstream.sh
python3 scripts/check-compatibility.py
scripts/verify-generated.sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo doc --no-deps --all-features

if [[ "${ALLOW_DIRTY:-0}" == "1" ]]; then
  cargo package --allow-dirty
else
  cargo package
fi
