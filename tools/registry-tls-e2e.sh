#!/usr/bin/env bash
# Disposable authenticated HTTPS registry. No external registry or credentials.
set -euo pipefail
cd "$(dirname "$0")/.."
[ "$(uname -s)" = Linux ]
for tool in registry openssl htpasswd curl skopeo podman jq buck2; do command -v "$tool" >/dev/null; done
test_dir=$(mktemp -d)
registry_pid=
cert_dir="$HOME/.config/containers/certs.d/localhost:5021"
owns_cert_dir=false
cleanup() {
    if [ -n "$registry_pid" ]; then kill "$registry_pid" 2>/dev/null || true; wait "$registry_pid" 2>/dev/null || true; fi
    if [ "$owns_cert_dir" = true ]; then rm -rf "$cert_dir"; fi
    rm -rf "$test_dir"
}
trap cleanup EXIT
umask 077
export REGISTRY_AUTH_FILE="$test_dir/auth.json"
export XDG_CONFIG_HOME="$test_dir/config"
mkdir -p "$XDG_CONFIG_HOME/containers" "$(dirname "$cert_dir")"
# Refuse to replace any existing trust configuration.
mkdir "$cert_dir"
owns_cert_dir=true
printf '%s\n' '{"default":[{"type":"insecureAcceptAnything"}]}' > "$XDG_CONFIG_HOME/containers/policy.json"
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=localhost \
    -addext subjectAltName=DNS:localhost -keyout "$test_dir/tls.key" -out "$cert_dir/ca.crt" 2>/dev/null
password=$(openssl rand -hex 32)
printf '%s\n' "$password" | htpasswd -Bni validation > "$test_dir/htpasswd"
cat > "$test_dir/registry.yaml" <<CONFIG
version: 0.1
log:
  level: error
storage:
  filesystem:
    rootdirectory: $test_dir/data
http:
  addr: 127.0.0.1:5021
  tls:
    certificate: $cert_dir/ca.crt
    key: $test_dir/tls.key
auth:
  htpasswd:
    realm: coldeer-validation
    path: $test_dir/htpasswd
CONFIG
registry serve "$test_dir/registry.yaml" > "$test_dir/registry.log" 2>&1 &
registry_pid=$!
ready=false
for attempt in {1..60}; do
    kill -0 "$registry_pid"
    code=$(curl --silent --cacert "$cert_dir/ca.crt" -o /dev/null -w '%{http_code}' https://localhost:5021/v2/ || true)
    if [ "$code" = 401 ]; then ready=true; break; fi
    sleep 1
done
[ "$ready" = true ]
printf '%s\n' "$password" | skopeo login --username validation --password-stdin localhost:5021
unset password
repository=localhost:5021/coldeer-validation
revision=$(git rev-parse HEAD)
buck2 run //demo:hello_container -- push "$repository" --tag validation --tag second
digest=$(skopeo inspect "docker://$repository:$revision" | jq -r .Digest)
for tag in validation second; do
    [ "$(skopeo inspect "docker://$repository:$tag" | jq -r .Digest)" = "$digest" ]
done
skopeo list-tags "docker://$repository" | jq -e --arg revision "$revision" '.Tags | sort == ([$revision,"validation","second"] | sort)'
podman pull --signature-policy "$XDG_CONFIG_HOME/containers/policy.json" "$repository:$revision"
[ "$(podman run --rm "$repository:$revision")" = 'Hello from Coldeer!' ]
printf 'Authenticated TLS registry passed: revision=%s digest=%s; three tags, no latest; pulled image ran\n' "$revision" "$digest"
