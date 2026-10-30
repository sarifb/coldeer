{ nixpkgs, planJSON }:
let
  pkgs = import (builtins.getFlake nixpkgs).outPath { };
  plan = builtins.fromJSON planJSON;
  references = map (p: builtins.storePath p) plan.references;
in
assert plan.schema_version == 1;
# CA outputs include Nix's own reference scan; candidates are scoped to those
# observed in the NAR by Rust. The staging source is not a runtime reference.
pkgs.runCommand plan.name
  {
    __contentAddressed = true;
    outputHashMode = "recursive";
    outputHashAlgo = "sha256";
    source = builtins.path {
      path = builtins.toPath plan.artifact;
      name = "coldeer-source";
    };
    inherit references;
    allowedReferences = references;
    preferLocalBuild = true;
  }
  ''
    cp -a "$source" "$out"
  ''
