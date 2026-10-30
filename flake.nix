{
  description = "Rust-first Buck2 to Nix packaging and layered containers";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/3e41b24abd260e8f71dbe2f5737d24122f972158";
  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      each = f: nixpkgs.lib.genAttrs systems (system: f (import nixpkgs { inherit system; }));
    in
    {
      packages = each (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "coldeer";
          version = "0.1.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
        };
      });
      devShells = each (
        pkgs:
        let
          # Intel Darwin's upstream igraph plotting tests pull in broken Arrow.
          # Coldeer uses no plotting; its real graph integration tests still run.
          graphPartitioner =
            if pkgs.stdenv.hostPlatform.system == "x86_64-darwin" then
              pkgs.flatten-references-graph.override {
                python3Packages = pkgs.python3Packages.overrideScope (
                  _final: prev: {
                    igraph = prev.igraph.overridePythonAttrs (_old: {
                      doCheck = false;
                      nativeCheckInputs = [ ];
                    });
                  }
                );
              }
            else
              pkgs.flatten-references-graph;
          toolchain = pkgs.writeTextDir "defs.bzl" ''
            # Generated from the locked Nixpkgs input. Do not edit.
            RUSTC = "${pkgs.rustc}/bin/rustc"
            RUSTDOC = "${pkgs.rustc}/bin/rustdoc"
            CLIPPY = "${pkgs.clippy}/bin/clippy-driver"
            CC = "${pkgs.llvmPackages.clang}/bin/clang"
            CXX = "${pkgs.llvmPackages.clang}/bin/clang++"
            AR = "${pkgs.llvmPackages.llvm}/bin/llvm-ar"
            LLD = "${pkgs.llvmPackages.lld}/bin/ld.lld"
            PYTHON = "${pkgs.python3}/bin/python3"
            RUST_NIX_PATHS = ${builtins.toJSON [ "${pkgs.rustc}" ]}
            CXX_NIX_PATHS = ${
              builtins.toJSON [
                "${pkgs.llvmPackages.clang}"
                "${pkgs.llvmPackages.llvm}"
                "${pkgs.llvmPackages.lld}"
                "${pkgs.stdenv.cc.libc}"
              ]
            }
            CXX_RUNTIME_LINK_FLAGS = ${
              builtins.toJSON (
                pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
                  "-Wl,-rpath,${pkgs.stdenv.cc.cc.lib}/lib"
                ]
              )
            }
          '';
        in
        {
          default = pkgs.mkShell {
            packages =
              with pkgs;
              [
                rustc
                cargo
                rustfmt
                clippy
                rust-analyzer
                buck2
                nix
                skopeo
                nix-prefetch-docker
                llvmPackages.clang
                llvmPackages.lld
                python3
                jq
                nixd
                nil
                graphPartitioner
                git
              ]
              ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.lima ]
              ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
                pkgs.podman
                pkgs.distribution
                pkgs.glibc.bin
              ];
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
            COLDEER_NIXPKGS = "path:${nixpkgs.outPath}";
            shellHook = ''
              coldeer_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
              if [ -f "$coldeer_root/toolchains/rust.bzl" ]; then
                mkdir -p "$coldeer_root/.coldeer"
                if [ "$(readlink "$coldeer_root/.coldeer/nix-toolchain" 2>/dev/null)" != "${toolchain}" ]; then
                  ln -sfn ${toolchain} "$coldeer_root/.coldeer/nix-toolchain"
                fi
              fi
              unset coldeer_root
            '';
          };
        }
      );
    };
}
