#!/usr/bin/env bash
# Operates only on the named test VM and its private Linux checkout.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
export LIMA_HOME="${COLDEER_VM_HOME:-${XDG_CACHE_HOME:-$HOME/.cache}/coldeer/lima}"
vm=${COLDEER_VM_NAME:-coldeer-test}
command -v limactl >/dev/null || { echo 'enter nix develop for limactl' >&2; exit 77; }
case "${1:-test}" in
  start)
    if limactl list --json | jq -e --arg name "$vm" 'select(.name == $name)' >/dev/null; then
      limactl start --tty=false --timeout=15m "$vm"
    else
      limactl start --tty=false --timeout=15m --name="$vm" "$repo/tools/linux-vm.yaml"
    fi
    limactl shell --workdir=/ "$vm" sudo test -f /var/lib/coldeer-validation/provisioned-2.34.7 || {
      echo 'Linux provisioning failed; inspect /var/log/cloud-init-output.log in the guest' >&2
      exit 1
    }
    ;;
  sync)
    # A fresh guest checkout avoids macOS binaries, shared Buck daemons, stale
    # files, and host-specific .coldeer toolchain links.
    limactl shell --workdir=/ "$vm" sudo bash -e -c '
      # Its working directory is about to be replaced. Match only this guest
      # checkout, never a daemon on the host or another guest project.
      pkill -f "^buck2d\\[workspace\\]" || true
      rm -rf /var/lib/coldeer-validation/workspace
      mkdir -p /var/lib/coldeer-validation/workspace
    '
    (cd "$repo" && COPYFILE_DISABLE=1 tar --no-xattrs --exclude='./.coldeer' --exclude='./buck-out' --exclude='./target' --exclude='*.json.guard' -cf - .) |
      limactl shell --workdir=/ "$vm" sudo tar --no-same-owner -xf - -C /var/lib/coldeer-validation/workspace
    ;;
  test)
    "$0" start
    "$0" sync
    mkdir -p "$repo/.coldeer"
    limactl shell --workdir=/ "$vm" sudo -H bash -lc '
      set -euo pipefail
      . /nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh
      cd /var/lib/coldeer-validation/workspace
      nix develop --command bash -e tools/linux-validation.sh
    ' 2>&1 | tee "$repo/.coldeer/linux-validation.log"
    ;;
  shell)
    exec limactl shell --workdir=/ "$vm" sudo -H bash -lc '
      . /nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh
      cd /var/lib/coldeer-validation/workspace
      exec nix develop
    '
    ;;
  stop) limactl stop "$vm" ;;
  *) echo 'usage: tools/linux-vm.sh [start|sync|test|shell|stop]' >&2; exit 2 ;;
esac
