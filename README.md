# Coldeer

Coldeer imports native Buck2 artifacts into the Nix store and builds layered
Linux containers from them. Its implementation is Rust and Starlark. Nix only
packages dependencies and constructs containers; Cargo is for editors and tests,
not Buck2 compilation. There are no third-party Rust dependencies, Haskell,
Mercury infrastructure, or project-owned Python sources.

## Setup

Install Nix, then enter `nix develop` (use `nix develop path:.` before adding
these files to Git). `flake.lock` pins the dependency set, including Rust 1.95.0
and Buck2. `rust-toolchain.toml` pins the same Rust release for rustup users.
macOS and Linux, each on ARM64 and x86-64, are supported as native build hosts.
macOS additionally needs the Xcode command-line tools.

The development shell includes rust-analyzer, rustfmt, and matching standard-library
sources via `RUST_SRC_PATH`. Launch your editor from this shell (or let its Nix/direnv
integration load it) so the language server inherits that environment. After changing
the shell, reload it and restart rust-analyzer. Format with `cargo fmt --all`.

Imports default to a Rust client speaking Nix's local daemon protocol (1.25 or
newer). It streams the NAR with its scanned references directly into the store.
Neither Lix, a builder, nor `ca-derivations` is needed for this backend. The
default socket is `/nix/var/nix/daemon-socket/socket`; `NIX_DAEMON_SOCKET_PATH`
and `NIX_REMOTE=unix:///path/to/socket` can select another local daemon.

The previous importer is retained: set `import_backend = "derivation"` on a
package or `--import_backend derivation` in BXL. For reference-bearing imports,
this backend stages a copy through a Nix CA derivation and requires the feature
enabled in both the client and daemon:

```ini
# /etc/nix/nix.conf; retain your other enabled features
experimental-features = nix-command flakes ca-derivations
```

Restart the daemon after changing its configuration. Coldeer enables the client
feature only when using this compatibility backend. It also remains available
for non-daemon store transports; direct imports never silently fall back to it.

Use plain `buck2` inside the development shell. On entry, Nix links the ignored
`.coldeer/nix-toolchain` cell to immutable definitions generated from `flake.lock`.
The native Rust and C++ toolchains use those pinned executable paths and expose
their Nix dependency roots directly through providers. No wrapper, environment
variable of candidate paths, or extra Buck2 configuration flags are needed.
Re-enter `nix develop` after changing the Nix lock to refresh the cell.
Buck2's bundled prelude uses its own Python compiler/linker helpers; their
interpreter is pinned by the same toolchain cell.

## Packages and arbitrary imports

```starlark
load("//rules:defs.bzl", "coldeer_package")

coldeer_package(
    name = "app_pkg",
    binaries = [":app"],        # one output per target; copied to bin/
    extra_files = {"share/config.json": "config.json"},
    nix_paths = [],              # additional Nix runtime candidate roots
)
```

```sh
buck2 build //demo:hello_pkg
package_file=$(buck2 build '//demo:hello_pkg[store_path]' --show-simple-output)
package=$(cat "$package_file")
"$package/bin/hello"
nix path-info --json "$package"

# A file or directory from any single-output target:
path_file=$(buck2 bxl //rules:import.bxl:main -- --target //demo:hello)
cat "$path_file"
```

The normal package output is a Buck2-built directory. `[store_path]` performs
an on-demand Nix import. BXL accepts multiple `--target` values and optional
`--nix_path` candidates, and prints one result-file path per configured target.
CLI modifiers are preserved. Multiple default outputs are rejected explicitly.

Package binary dependencies whose linking configuration is automatic transition
to static PIC linkage. Explicit shared/static configurations and target
`link_style` attributes are preserved. This bundles Buck-built dependency
libraries into the executable; it does not force static linkage of glibc or
macOS system libraries. Files supplied through `extra_files` are not transitioned.

Coldeer gathers candidates from providers in the configured Buck dependency
graph, expands their Nix closures, streams a NAR through its reference scanner,
and creates a content-addressed Nix output with only the observed references.
Dependencies retain their original store paths; their closures are not rewritten.
A sibling `.gcroot` symlink keeps imported paths alive while Buck outputs remain.
`buck2 clean` removes those roots along with the outputs.

External Nix integrations can expose `ColdeerNixCandidates` or `ColdeerNixInfo`
from `rules/providers.bzl`, or use `coldeer_nix_dependency(path = "/nix/store/…")`.
Provide every possible Nix dependency through those providers or `nix_paths`.
Non-Nix shared libraries are not automatically relocated or bundled. Use static
application linkage or explicitly package the needed shared libraries. A missing
candidate cannot be inferred from a build graph that does not describe it.

To recover a Nix string context in another expression:

```nix
builtins.storePath "/nix/store/…-app_pkg"
```

## Containers

```starlark
load("//rules:defs.bzl", "coldeer_container")

coldeer_container(
    name = "app_container",
    description = "My application",
    cmd = ["/bin/app"],
    main_contents = [":app_pkg"],
    contents = [],
    env = {"PORT": "8080"},
    ports = ["8080/tcp"],
    labels = {"org.opencontainers.image.licenses": "MIT"},
    system = "aarch64-linux",  # or x86_64-linux; defaults to host CPU + Linux
    # base_image = ":alpine",
    # user = "1000:1000",
    # working_dir = "/",
)
```

```sh
buck2 run //demo:hello_container -- plan
# On a Linux build host, or with a configured Linux Nix builder:
buck2 run //demo:hello_container -- build ./image-stream
buck2 run //demo:hello_container -- save image.tar
# No filename means stream only archive bytes to stdout:
buck2 run //demo:hello_container -- save | podman load
# Requires an existing Git HEAD; use your normal skopeo registry credentials:
buck2 run //demo:hello_container -- push ghcr.io/account/app --tag production
```

`build` leaves a GC-rooted image-streaming executable at the requested path.
`save` builds and runs it; an existing destination file is replaced only after
successful streaming. `push` uploads the **full Git revision tag plus each
explicit `--tag`**, deduplicated. It does not implicitly push `latest`. Local
archives have a `latest` image tag for convenient loading. Push errors propagate;
if a later tag fails, earlier registry writes may already have succeeded.
Push pipes the image directly into one skopeo upload, without staging an image
archive in Coldeer. Additional tags are created from the returned immutable registry
digest. Both producer and uploader failures stop retagging.

OCI labels include description, title, source, revision, and version. User labels
are retained except for these generated fields. Builds outside a committed Git
checkout use `uncommitted` metadata; pushes require a real revision. Build time
is not injected, so repeated builds of identical inputs remain reproducible.
The generated `GIT_SHA1` environment variable matches the revision labels and
overrides any user-supplied value for that key.
Images include tini as PID 1, CA certificates, `/usr/bin/env`, Bash, coreutils,
curl, a UTF-8 locale environment, and a writable sticky `/tmp`. Each application
root gets its own layer. Platform tools, application dependency closures, and
additional contents have independent partitions. Dependency populations use
popularity ranking and separate 90/30 layer budgets (reduced for large numbers
of application roots), so changes to diagnostics do not repartition app deps.
At most 119 application roots are accepted.

**Linux images require a Linux builder and Linux application binaries.** A
macOS binary does not become Linux-compatible when placed into an image.
Build the demo on Linux for a runnable Linux image. A macOS host can use a
configured remote Nix builder to construct images containing already-built Linux
packages. Coldeer does not silently cross-compile or configure remote machines.

## Base images

`demo:alpine` is a working two-architecture example with pins in
`demo/alpine.lock.json`. Run `buck2 run //demo:alpine -- update` to refresh
them or `buck2 build //demo:alpine` to validate the active architecture.

Pins belong in a checked-in JSON file, never in tool-rewritten BUCK source.
Start with `images/alpine.lock.json`:

```json
{
  "schema_version": 1,
  "image_name": "docker.io/library/alpine",
  "image_digest": "",
  "nar_hashes": {}
}
```

```starlark
load("//rules:defs.bzl", "coldeer_base_image")
coldeer_base_image(
    name = "alpine",
    image_name = "docker.io/library/alpine",
    follows_tag = "3.23",
    lock_file = "images/alpine.lock.json",
    systems = ["aarch64-linux", "x86_64-linux"],
)
```

```sh
buck2 run :alpine -- update   # resolve tag digest, prefetch every architecture
buck2 run :alpine -- lock    # retain digest, recompute architecture hashes
buck2 build :alpine          # pull and validate the active system's pin
```

Prefetch uses native skopeo to download each requested architecture. Validation
may require the corresponding Linux builder when the archive is not already
available in the store or a substituter. One manifest-list
digest is shared across architectures, with a SHA256 SRI archive hash per Nix system
(the `nar_hashes` lock field).
Final image name and tag (`pinned`) match between prefetch and pull. Updates hold
an exclusive sibling `.json.guard`, compute and validate every requested pin,
then fsync and atomically rename a same-directory temporary file. Failed updates
leave the original lock unchanged. `lock` never resolves the moving tag. After
an interrupted process, inspect and remove a stale guard before retrying.

## Validation

GitHub Actions runs native checks on standard macOS and Linux runners for both
ARM64 and x86-64. Linux uses rootless Podman. Separate Linux integration jobs
test a disposable authenticated TLS registry and an SSH Nix builder with an
independent container store and local builds disabled. These jobs run only for
public repositories, where standard runners are free. They do not require
registry secrets or publish images externally. A separate, explicitly triggered
[GHCR experiment](.github/workflows/registry-ghcr.yml) uses the temporary GitHub
token to push/pull a unique external test package and then deletes that package.
See the
[native workflow](.github/workflows/validation.yml) and
[Linux experiments](.github/workflows/remote-builder.yml).

On macOS, the development shell includes Lima. Run the full Linux suite in a
native Linux VM using Apple's virtualization framework:

```sh
nix develop
tools/linux-vm.sh test
tools/linux-vm.sh shell   # Linux development shell in the guest checkout
tools/linux-vm.sh stop
```

The VM uses 4 CPUs, 6 GiB RAM, and a sparse 32 GiB disk. Ubuntu images and the
Nix installer are checksum-pinned in `tools/linux-vm.yaml`. Initial provisioning
requires network access. VM data lives outside the repository at
`${XDG_CACHE_HOME:-$HOME/.cache}/coldeer/lima`; override with `COLDEER_VM_HOME`,
and select another instance with `COLDEER_VM_NAME`. No host directories are
mounted. `test` replaces the guest's private checkout with the current working
tree, including its Git history, and runs tests as root inside the disposable
guest. Host build outputs and toolchain links are excluded. Logs are saved to
`.coldeer/linux-validation.log`. Changes in the guest are not copied back.

This suite runs Cargo and native Buck2 tests, both import backends, real daemon
and graph-partitioner tests, C++ linkage checks, image save/load/run, OCI metadata
and non-root runtime checks, a pinned Alpine base, and layer-cache comparisons.
A temporary loopback registry receives streamed uploads; revision and explicitly
requested tags must share a digest, and unchanged layers must retain their
compressed registry digests. The registry is stopped and deleted afterward.
Unsigned local images and loopback HTTP are enabled only in the test session's
temporary containers configuration. No external registry receives a push.

On Linux, enter `nix develop` and run `tools/linux-validation.sh` directly with
a working Podman setup. Individual checks remain available:

```sh
cargo test --locked
cargo test --locked --workspace   # also compile the demo's editor workspace
buck2 test //:unit-tests
tools/e2e.sh                     # both import backends; CA daemon needed for derivation
cargo test --test layers -- --ignored  # real Nixpkgs layer-partitioner tests
cargo test --lib real_daemon_without_ca_derivations -- --ignored
COLDEER_TEST_CONTAINERS=1 tools/e2e.sh  # additionally save/load/run with podman on Linux
```

Process tests use isolated fake tools and never contact a registry. The optional
container test runs only when explicitly requested. See [validation results](docs/validation.md)
for checks actually run in this workspace and [JSON contracts](docs/interfaces.md)
for direct CLI use. `COLDEER_NIXPKGS` can override the locked Nixpkgs flake for
integration testing; default CLI builds use both the revision and NAR hash from
this repository's embedded `flake.lock`.
