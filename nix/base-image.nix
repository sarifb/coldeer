{ nixpkgs, planJSON }:
let
  plan = builtins.fromJSON planJSON;
  pkgs = import (builtins.getFlake nixpkgs).outPath { system = plan.system; };
  base = plan.base_image;
in
assert plan.schema_version == 1;
assert builtins.elem plan.system [
  "aarch64-linux"
  "x86_64-linux"
];
pkgs.dockerTools.pullImage {
  imageName = base.image_name;
  imageDigest = base.image_digest;
  hash = base.hash;
  os = "linux";
  arch = if plan.system == "aarch64-linux" then "arm64" else "amd64";
  finalImageName = base.image_name;
  finalImageTag = "pinned";
}
