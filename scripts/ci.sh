#!/usr/bin/env bash
#
# Local CI gate — mirrors the checks that .github/workflows/ci.yml used to run
# in the cloud (now disabled / manual-only to avoid GitHub Actions cost).
#
# Run this before pushing to main:
#
#     ./scripts/ci.sh
#
# It uses the toolchain pinned in rust-toolchain.toml (1.91.0). Requires
# `protoc` on PATH — lance-encoding's build.rs generates Rust from .proto:
#   macOS:  brew install protobuf
#   Debian: apt-get install -y protobuf-compiler
#
# Exit code is non-zero on the first failing step (fail-fast), so a clean
# run means the same gate the cloud used to enforce is green locally.

set -euo pipefail

# Run from the repo root regardless of where the script is invoked.
cd "$(dirname "$0")/.."

if ! command -v protoc >/dev/null 2>&1; then
  echo "ERROR: protoc not found on PATH — lance-encoding's build.rs needs it." >&2
  echo "  macOS:  brew install protobuf" >&2
  echo "  Debian: apt-get install -y protobuf-compiler" >&2
  exit 1
fi

echo "==> cargo fmt --all --check"
cargo fmt --all --check

echo "==> cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo test --workspace"
cargo test --workspace

echo "==> cargo doc --workspace --no-deps (warnings = errors)"
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

echo
echo "OK: local CI gate passed — safe to push."
