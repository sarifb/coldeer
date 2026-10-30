#!/usr/bin/env bash
# Native CI entry point. Linux intentionally runs as the unprivileged runner.
set -euo pipefail
cd "$(dirname "$0")/.."
uname -a
git rev-parse HEAD
nix --version
rustc --version
buck2 --version
cargo fmt --all -- --check
case "$(uname -s)" in
    Linux)
        [ "$(id -u)" != 0 ]
        podman info --format json | jq -e '.host.security.rootless == true'
        tools/linux-validation.sh
        ;;
    Darwin)
        cargo test --locked --workspace
        cargo clippy --locked --workspace --all-targets -- -D warnings
        buck2 test //:unit-tests
        tools/e2e.sh
        cargo test --locked --test layers -- --ignored
        cargo test --locked real_daemon_without_ca_derivations -- --ignored
        ;;
    *) exit 77 ;;
esac
