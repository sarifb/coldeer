#!/usr/bin/env bash
# Actual Linux images and a private loopback registry; no external pushes.
set -euo pipefail
cd "$(dirname "$0")/.."
[ "$(uname -s)" = Linux ]
for tool in buck2 nix jq podman skopeo registry; do command -v "$tool" >/dev/null; done
test_dir=$(mktemp -d)
registry_pid=
cleanup() {
    if [ -n "$registry_pid" ]; then kill "$registry_pid" 2>/dev/null || true; wait "$registry_pid" 2>/dev/null || true; fi
    rm -rf "$test_dir"
}
trap cleanup EXIT
export XDG_CONFIG_HOME="$test_dir/config"
mkdir -p "$XDG_CONFIG_HOME/containers"
printf '%s\n' '{"default":[{"type":"insecureAcceptAnything"}]}' > "$XDG_CONFIG_HOME/containers/policy.json"
export COLDEER_TEST_SIGNATURE_POLICY="$XDG_CONFIG_HOME/containers/policy.json"
podman() {
    case "$1" in
        load|pull) command podman "$1" --signature-policy "$COLDEER_TEST_SIGNATURE_POLICY" "${@:2}" ;;
        *) command podman "$@" ;;
    esac
}
cli=$(buck2 build //:coldeer --show-simple-output)
buck2 run //demo:hello_container -- plan > "$test_dir/original.json"
revision=$(git rev-parse HEAD)
# Exercise runtime metadata as well as non-root image configuration.
jq '.name="coldeer-validation" | .env.CHECK="present" | .ports=["8080/tcp","5353/udp"] | .working_dir="/tmp" | .user="65534:65534"' \
    "$test_dir/original.json" > "$test_dir/runtime.json"
stream=$("$cli" container "$test_dir/runtime.json" build "$test_dir/stream")
[ "$(readlink -f "$test_dir/stream")" = "$stream" ]
"$stream" | podman load
image=localhost/coldeer-validation:latest
[ "$(podman run --rm "$image")" = 'Hello from Coldeer!' ]
podman image inspect "$image" > "$test_dir/inspect.json"
jq -e --arg revision "$revision" '.[0].Config |
    .Labels["org.opencontainers.image.revision"] == $revision and
    .Labels["org.opencontainers.image.version"] == $revision and
    (.Env | index("GIT_SHA1="+$revision) != null) and
    (.Env | index("CHECK=present") != null) and
    (.ExposedPorts | has("8080/tcp") and has("5353/udp")) and
    .User == "65534:65534" and .WorkingDir == "/tmp"' "$test_dir/inspect.json" >/dev/null
podman run --rm "$image" /bin/bash -ec '
    [ "$GIT_SHA1" = "$1" ]; [ "$CHECK" = present ]; [ "$LANG" = C.UTF-8 ]
    [ "$(id -u)" = 65534 ]; [ "$(pwd)" = /tmp ]; [ "$(stat -c %a /tmp)" = 1777 ]
    case "$(readlink /proc/1/exe)" in */bin/tini) ;; *) exit 1;; esac
    touch /tmp/writable
' check "$revision"
# Add a real pinned base, preserving the Linux executable built by Buck2.
jq --arg lock "$PWD/demo/alpine.lock.json" '.name="coldeer-alpine" | .base_lock_file=$lock' \
    "$test_dir/original.json" > "$test_dir/base.json"
"$cli" container "$test_dir/base.json" save | podman load
[ "$(podman run --rm localhost/coldeer-alpine:latest)" = 'Hello from Coldeer!' ]
[ "$(podman run --rm localhost/coldeer-alpine:latest /bin/busybox echo base-works)" = base-works ]
# Change only application contents; verify actual archive layer digests, not
# merely the graph partitioner's output. Both packages use identical runtimes.
original=$(jq -r '.main_contents[0]' "$test_dir/original.json")
nix-store --query --requisites "$original" | jq -Rsc 'split("\n") | map(select(length>0))' > "$test_dir/candidates.json"
for version in one two; do
    mkdir -p "$test_dir/$version/artifact"
    cp -RL "$original/." "$test_dir/$version/artifact/"
    chmod -R u+w "$test_dir/$version/artifact"
    mkdir -p "$test_dir/$version/artifact/share"
    printf '%s\n' "$version" > "$test_dir/$version/artifact/share/version"
    jq -n --arg artifact "$test_dir/$version/artifact" --slurpfile paths "$test_dir/candidates.json" \
        '{schema_version:1,name:"validation-app",artifact:$artifact,candidate_paths:$paths[0]}' > "$test_dir/$version/import.json"
    "$cli" import "$test_dir/$version/import.json" "$test_dir/$version/path"
    jq --arg path "$(cat "$test_dir/$version/path")" \
        'del(.main_path_files) | .main_contents=[$path] | .name="coldeer-cache"' \
        "$test_dir/original.json" > "$test_dir/$version/plan.json"
    "$cli" container "$test_dir/$version/plan.json" save "$test_dir/$version/image.tar"
    mkdir "$test_dir/$version/archive"
    tar -xf "$test_dir/$version/image.tar" -C "$test_dir/$version/archive"
    jq -r '.[0].Layers[]' "$test_dir/$version/archive/manifest.json" |
        while IFS= read -r layer; do sha256sum "$test_dir/$version/archive/$layer" | cut -d' ' -f1; done > "$test_dir/$version/hashes"
done
common=$(comm -12 <(sort "$test_dir/one/hashes") <(sort "$test_dir/two/hashes") | wc -l)
[ "$common" -gt 0 ]
! cmp -s "$test_dir/one/hashes" "$test_dir/two/hashes"
# Every layer that contains neither changed application root nor its rootfs
# symlink must be byte-identical across the two images.
app_one=$(basename "$(cat "$test_dir/one/path")")
app_two=$(basename "$(cat "$test_dir/two/path")")
while IFS= read -r layer; do
    archive="$test_dir/one/archive/$layer"
    if ! tar -tvf "$archive" | grep -F "$app_one" >/dev/null; then
        hash=$(sha256sum "$archive" | cut -d' ' -f1)
        grep -Fx "$hash" "$test_dir/two/hashes" >/dev/null
    fi
done < <(jq -r '.[0].Layers[]' "$test_dir/one/archive/manifest.json")
echo "Layer caching: $common identical layers across changed application roots $app_one / $app_two"
# Local HTTP is confined to this test process's containers configuration.
cat > "$XDG_CONFIG_HOME/containers/registries.conf" <<'CONFIG'
[[registry]]
location = "127.0.0.1:5019"
insecure = true
CONFIG
cat > "$test_dir/registry.yaml" <<CONFIG
version: 0.1
log:
  level: error
storage:
  filesystem:
    rootdirectory: $test_dir/registry
http:
  addr: 127.0.0.1:5019
CONFIG
registry serve "$test_dir/registry.yaml" > "$test_dir/registry.log" 2>&1 &
registry_pid=$!
for attempt in $(seq 1 100); do
    if curl --fail --silent http://127.0.0.1:5019/v2/ >/dev/null; then break; fi
    kill -0 "$registry_pid"
    sleep 0.1
done
curl --fail --silent http://127.0.0.1:5019/v2/ >/dev/null
repo=127.0.0.1:5019/coldeer-validation
"$cli" container "$test_dir/runtime.json" push "$repo" --tag integration --tag second
for tag in "$revision" integration second; do
    skopeo inspect "docker://$repo:$tag" > "$test_dir/$tag.json"
done
[ "$(jq -r .Digest "$test_dir/$revision.json")" = "$(jq -r .Digest "$test_dir/integration.json")" ]
[ "$(jq -r .Digest "$test_dir/$revision.json")" = "$(jq -r .Digest "$test_dir/second.json")" ]
! skopeo inspect "docker://$repo:latest" > /dev/null 2>&1
podman pull --tls-verify=false "$repo:integration"
[ "$(podman run --rm "$repo:integration")" = 'Hello from Coldeer!' ]
for version in one two; do
    "$cli" container "$test_dir/$version/plan.json" push "$repo-cache" --tag "$version"
    skopeo inspect "docker://$repo-cache:$version" | jq -r '.Layers[]' > "$test_dir/$version/compressed"
done
compressed_common=$(comm -12 <(sort "$test_dir/one/compressed") <(sort "$test_dir/two/compressed") | wc -l)
[ "$compressed_common" -eq "$common" ]
! cmp -s "$test_dir/one/compressed" "$test_dir/two/compressed"
echo "Registry caching: $compressed_common identical compressed layer digests"
echo 'Real container runtime, metadata, base-image, layer caching, streaming push, revision and explicit tag checks passed'
