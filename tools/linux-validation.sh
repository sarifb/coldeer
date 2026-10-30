#!/usr/bin/env bash
# Run inside a disposable Linux guest, with the flake development shell active.
set -euo pipefail
cd "$(dirname "$0")/.."
[ "$(uname -s)" = Linux ]
test_config=$(mktemp -d)
trap 'rm -rf "$test_config"' EXIT
export XDG_CONFIG_HOME="$test_config"
mkdir -p "$XDG_CONFIG_HOME/containers"
# Only this disposable test session accepts the unsigned local test images.
printf '%s\n' '{"default":[{"type":"insecureAcceptAnything"}]}' > "$XDG_CONFIG_HOME/containers/policy.json"
export COLDEER_TEST_SIGNATURE_POLICY="$XDG_CONFIG_HOME/containers/policy.json"
podman() {
    case "$1" in
        load|pull) command podman "$1" --signature-policy "$COLDEER_TEST_SIGNATURE_POLICY" "${@:2}" ;;
        *) command podman "$@" ;;
    esac
}
export -f podman
uname -a
nix --version
rustc --version
buck2 --version
git rev-parse HEAD
cargo test --locked --workspace
cargo test --locked --test layers -- --ignored
cargo test --locked real_daemon_without_ca_derivations -- --ignored
cargo clippy --locked --workspace --all-targets -- -D warnings
buck2 test //:unit-tests
buck2 build //demo:alpine
COLDEER_TEST_CONTAINERS=1 tools/e2e.sh
tools/container-e2e.sh
echo 'Linux validation passed'
