#!/usr/bin/env bash
# Explicit integration test. Never pushes to an external registry.
set -euo pipefail
cd "$(dirname "$0")/.."
for tool in buck2 nix jq; do command -v "$tool" >/dev/null || { echo "missing $tool; enter nix develop" >&2; exit 77; }; done
test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT
cli=$(buck2 build //:coldeer --show-simple-output)
buck2 bxl //tests:toolchains.bxl:main -- --target //demo:hello_pkg
package_file=$(buck2 build '//demo:hello_pkg[store_path]' --show-simple-output)
package=$(cat "$package_file")
[ "$("$package/bin/hello")" = 'Hello from Coldeer!' ]
path_file=$(buck2 bxl //rules:import.bxl:main -- --target //demo:hello)
[ "$("$(cat "$path_file")")" = 'Hello from Coldeer!' ]
legacy_file=$(buck2 bxl //rules:import.bxl:main -- --target //demo:hello --import_backend derivation)
cmp "$path_file" "$legacy_file"
static_pkg=$(buck2 build //tests/linking:static_pkg --show-simple-output)
shared_pkg=$(buck2 build //tests/linking:shared_pkg --show-simple-output)
configured_shared_pkg=$(buck2 build //tests/linking:static_pkg --target-platforms //tests/linking:shared_host --show-simple-output)
[ "$("$static_pkg/bin/app")" = 'Static package linkage works' ]
case "$(uname -s)" in
    Darwin)
        static_libraries=$(otool -L "$static_pkg/bin/app")
        shared_libraries=$(otool -L "$shared_pkg/bin/shared_app")
        configured_libraries=$(otool -L "$configured_shared_pkg/bin/app")
        ;;
    Linux)
        static_libraries=$(ldd "$static_pkg/bin/app")
        shared_libraries=$(ldd "$shared_pkg/bin/shared_app")
        configured_libraries=$(ldd "$configured_shared_pkg/bin/app")
        ;;
esac
case "$static_libraries" in *greeting*) echo 'package retained a Buck-built shared dependency' >&2; exit 1;; esac
case "$shared_libraries" in *greeting*) ;; *) echo 'explicit shared linkage was overridden' >&2; exit 1;; esac
case "$configured_libraries" in *greeting*) ;; *) echo 'explicit platform linkage was overridden' >&2; exit 1;; esac
printf 'dependency\n' > "$test_dir/dependency"
printf 'unused\n' > "$test_dir/unused"
dep=$(nix store add-path "$test_dir/dependency")
unused=$(nix store add-path "$test_dir/unused")
mkdir "$test_dir/artifact"
printf 'reference=%s\n' "$dep" > "$test_dir/artifact/data"
jq -n --arg artifact "$test_dir/artifact" --arg dep "$dep" --arg unused "$unused" --arg name "reference-$(basename "$test_dir")" \
    '{schema_version:1,name:$name,artifact:$artifact,candidate_paths:[$dep,$unused]}' > "$test_dir/import.json"
"$cli" import "$test_dir/import.json" "$test_dir/imported"
imported=$(cat "$test_dir/imported")
[ "$(nix-store --query --references "$imported")" = "$dep" ]
cmp "$test_dir/artifact/data" "$imported/data"
nix path-info --json "$imported" > "$test_dir/info.json"
jq -e '.. | objects | select(has("ca")) | .ca | startswith("fixed:r:sha256:")' "$test_dir/info.json" >/dev/null
# A repeated import must have exactly the same content address.
"$cli" import "$test_dir/import.json" "$test_dir/imported-again"
cmp "$test_dir/imported" "$test_dir/imported-again"
# The retained derivation backend must produce the same NAR, references, and address.
jq '.import_backend = "derivation"' "$test_dir/import.json" > "$test_dir/derivation.json"
"$cli" import "$test_dir/derivation.json" "$test_dir/imported-derivation"
cmp "$test_dir/imported" "$test_dir/imported-derivation"
# Direct imports also work without enabling CA derivations on the client.
NIX_CONFIG='experimental-features = nix-command flakes' "$cli" import "$test_dir/import.json" "$test_dir/imported-direct"
cmp "$test_dir/imported" "$test_dir/imported-direct"
if [ "${COLDEER_TEST_CONTAINERS:-0}" = 1 ]; then
    [ "$(uname -s)" = Linux ] || { echo 'container demo requires Linux application binaries' >&2; exit 77; }
    command -v podman >/dev/null || { echo 'podman is required for container tests' >&2; exit 77; }
    buck2 run //demo:hello_container -- save "$test_dir/image.tar"
    podman load -i "$test_dir/image.tar"
    [ "$(podman run --rm localhost/hello_container:latest)" = 'Hello from Coldeer!' ]
fi
echo 'Coldeer end-to-end checks passed' >&2
