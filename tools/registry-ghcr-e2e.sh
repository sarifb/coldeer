#!/usr/bin/env bash
# Explicit external-registry check; invoked only by the GHCR experiment workflow.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${COLDEER_REGISTRY_TOKEN:?workflow token required}"
: "${GITHUB_ACTOR:?workflow actor required}"
repository=${1:?pass a disposable ghcr.io repository}
case "$repository" in ghcr.io/*/*) ;; *) echo 'expected a disposable GHCR repository' >&2; exit 2;; esac
test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT
umask 077
export REGISTRY_AUTH_FILE="$test_dir/auth.json"
printf '%s\n' "$COLDEER_REGISTRY_TOKEN" | skopeo login --username "$GITHUB_ACTOR" --password-stdin ghcr.io
unset COLDEER_REGISTRY_TOKEN
revision=$(git rev-parse HEAD)
buck2 run //demo:hello_container -- push "$repository" --tag validation --tag second
digest=$(skopeo inspect "docker://$repository:$revision" | jq -r .Digest)
for tag in validation second; do
    [ "$(skopeo inspect "docker://$repository:$tag" | jq -r .Digest)" = "$digest" ]
done
skopeo list-tags "docker://$repository" | jq -e --arg revision "$revision" '.Tags | sort == ([$revision,"validation","second"] | sort)'
printf '%s\n' '{"default":[{"type":"insecureAcceptAnything"}]}' > "$test_dir/policy.json"
podman pull --signature-policy "$test_dir/policy.json" "$repository:$revision"
[ "$(podman run --rm "$repository:$revision")" = 'Hello from Coldeer!' ]
printf 'External GHCR authenticated TLS passed: revision=%s digest=%s; three tags, no latest; pulled image ran\n' "$revision" "$digest"
