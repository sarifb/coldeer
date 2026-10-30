#!/usr/bin/env bash
# SSH builder in a separate container/store, on the same Linux CI host.
set -euo pipefail
cd "$(dirname "$0")/.."
[ "$(uname -s)" = Linux ]
: "${COLDEER_NIXPKGS:?enter nix develop}"
test_dir=$(mktemp -d)
builder_container=
cleanup() {
    if [ -n "$builder_container" ]; then podman rm -f "$builder_container" >/dev/null; fi
    rm -rf "$test_dir"
}
trap cleanup EXIT
mkdir -p "$test_dir/ssh"
ssh-keygen -q -t ed25519 -N '' -f "$test_dir/client"
ssh-keygen -q -t ed25519 -N '' -f "$test_dir/ssh/host"
cp "$test_dir/client.pub" "$test_dir/ssh/authorized_keys"
cat > "$test_dir/ssh/sshd_config" <<'CONFIG'
Port 22
HostKey /config/host
AuthorizedKeysFile /config/authorized_keys
PermitRootLogin prohibit-password
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM no
StrictModes no
PidFile /run/sshd.pid
CONFIG
nix build --impure --file nix/ci-builder.nix --argstr nixpkgs "$COLDEER_NIXPKGS" --out-link "$test_dir/builder-image"
printf '%s\n' '{"default":[{"type":"insecureAcceptAnything"}]}' > "$test_dir/policy.json"
podman load --signature-policy "$test_dir/policy.json" -i "$test_dir/builder-image"
builder_container=$(podman run -d -p 127.0.0.1:5022:22 -v "$test_dir/ssh:/config:ro" localhost/coldeer-ci-builder:test)
host_key=$(base64 < "$test_dir/ssh/host.pub" | tr -d '\n')
encoded_host_key=$(printf '%s' "$host_key" | sed 's/+/%2B/g; s,/,\%2F,g; s/=/%3D/g')
remote="ssh://root@127.0.0.1:5022?ssh-key=$test_dir/client&base64-ssh-public-host-key=$encoded_host_key"
ready=false
for _attempt in {1..60}; do
    if nix store ping --store "$remote" >/dev/null 2>&1; then ready=true; break; fi
    sleep 1
done
if [ "$ready" != true ]; then podman logs "$builder_container"; exit 1; fi
cli=$(buck2 build //:coldeer --show-simple-output)
buck2 run //demo:hello_container -- plan > "$test_dir/original.json"
# A unique label forces a fresh image derivation rather than a local cache hit.
jq --arg nonce "$(basename "$test_dir")" '.metadata["org.coldeer.remote-test"]=$nonce' "$test_dir/original.json" > "$test_dir/plan.json"
system=$(nix eval --impure --raw --expr builtins.currentSystem)
# Disable all local builds for the actual Coldeer image construction.
export NIX_CONFIG="${NIX_CONFIG:-}
max-jobs = 0
builders = ssh://root@127.0.0.1:5022 $system $test_dir/client 1 1 - - $host_key
"
stream=$("$cli" container "$test_dir/plan.json" build "$test_dir/stream")
unset NIX_CONFIG
# The result must exist in the independent builder store as well as on the host.
podman exec "$builder_container" nix-store --check-validity "$stream"
"$stream" | podman load --signature-policy "$test_dir/policy.json"
[ "$(podman run --rm localhost/hello_container:latest)" = 'Hello from Coldeer!' ]
printf 'SSH Linux builder passed: local builds disabled; independent store produced %s; loaded image ran\n' "$stream"
