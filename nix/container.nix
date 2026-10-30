{ nixpkgs, planJSON }:
let
  plan = builtins.fromJSON planJSON;
  pkgs = import (builtins.getFlake nixpkgs).outPath { system = plan.system; };
  lib = pkgs.lib;
  store = p: builtins.storePath p;
  contents = map store plan.contents;
  main = map store plan.main_contents;
  base = plan.base_image or null;
  fromImage =
    if base == null then
      null
    else
      pkgs.dockerTools.pullImage {
        imageName = base.image_name;
        imageDigest = base.image_digest;
        hash = base.hash;
        os = "linux";
        arch = if plan.system == "aarch64-linux" then "arm64" else "amd64";
        finalImageName = base.image_name;
        finalImageTag = "pinned";
      };
  tools = [
    pkgs.dockerTools.usrBinEnv
    pkgs.dockerTools.caCertificates
    pkgs.bash
    pkgs.coreutils
    pkgs.curl
  ];
in
assert plan.schema_version == 1;
assert builtins.elem plan.system [
  "aarch64-linux"
  "x86_64-linux"
];
assert builtins.length main < 120;
pkgs.dockerTools.streamLayeredImage {
  name = plan.name;
  tag = "latest";
  inherit fromImage;
  contents = tools ++ contents ++ main;
  maxLayers = 127;
  # Platform tools form a stable partition before application dependency layering.
  layeringPipeline = [
    [
      "subcomponent_out"
      (tools ++ [ pkgs.tini ])
    ]
  ]
  ++ plan.layering_pipeline;
  extraCommands = ''
    mkdir -p tmp
    chmod 1777 tmp
  '';
  config = {
    Cmd = plan.cmd;
    Entrypoint = [
      "${pkgs.tini}/bin/tini"
      "--"
    ];
    Env = lib.mapAttrsToList (k: v: "${k}=${v}") plan.env;
    ExposedPorts = lib.genAttrs (plan.ports or [ ]) (_: { });
    Labels = plan.metadata;
    WorkingDir = plan.working_dir or "/";
    User = plan.user or "";
  };
}
