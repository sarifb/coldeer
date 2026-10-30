{ nixpkgs }:
let
  pkgs = import (builtins.getFlake nixpkgs).outPath { };
in
pkgs.dockerTools.buildLayeredImage {
  name = "coldeer-ci-builder";
  tag = "test";
  maxLayers = 8;
  includeNixDB = true;
  contents = [
    pkgs.nix
    pkgs.openssh
    pkgs.bash
    pkgs.coreutils
    pkgs.cacert
  ];
  extraCommands = ''
    mkdir -p etc/nix root tmp run/sshd var/empty
    chmod 1777 tmp
    printf '%s\n' 'root::0:0:root:/root:/bin/bash' 'sshd:x:74:74:sshd:/var/empty:/bin/bash' > etc/passwd
    printf '%s\n' 'root:x:0:' 'sshd:x:74:' > etc/group
    cat > etc/nix/nix.conf <<'CONFIG'
    experimental-features = nix-command flakes
    sandbox = false
    build-users-group =
    CONFIG
  '';
  config = {
    Env = [
      "PATH=/bin"
      "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
    ];
    Cmd = [
      "${pkgs.openssh}/bin/sshd"
      "-D"
      "-e"
      "-f"
      "/config/sshd_config"
    ];
  };
}
