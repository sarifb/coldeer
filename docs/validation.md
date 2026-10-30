# Validation performed

## Current-repository CI results

Experiments run on 2026-10-30 from this repository's Git history. Standard
GitHub-hosted runners are used only when the repository is public. Each native
log records its Git revision, platform, architecture, and tool versions. CI
installs Nix 2.34.7 and uses the locked Rust 1.95.0 and Nixpkgs dependencies.
Artifacts are retained for seven days; the run links also identify the tested
commits independently of later documentation changes.

The [native validation run](https://github.com/sarifb/coldeer/actions/runs/41082035765)
passed at `7e0470ef857a6b8f8caf27086058edf83a45d463`:

| Native platform | Runner | Outcome |
| --- | --- | --- |
| Linux x86-64 | `ubuntu-24.04` | Passed, including rootless containers |
| Linux ARM64 | `ubuntu-24.04-arm` | Passed, including rootless containers |
| macOS ARM64 | `macos-15` | Passed |
| macOS x86-64 | `macos-15-intel` | Passed |

Native validation uses `nix develop --command bash tools/ci-validation.sh`:

- Every platform runs formatting, Cargo workspace tests, Clippy with warnings
  denied, native Buck2 tests, both package/BXL import backends, linkage checks,
  and the explicitly enabled real-daemon and graph-partitioner tests.
- Linux runs `tools/linux-validation.sh` as the ordinary runner user, verifies
  `podman info` reports rootless mode, and exercises build/save/load/run, OCI
  metadata, non-root image execution, Alpine pins, HTTP loopback streaming push,
  revision and explicit tags, and compressed layer-cache reuse. Ubuntu's
  AppArmor restriction on unprivileged user namespaces is disabled in these
  disposable runners so rootless Podman can create its namespaces.

The [Linux integration run](https://github.com/sarifb/coldeer/actions/runs/41081446260)
passed at `24914404eed89ef3bda4395649f94762f0b7fa68`:

- `nix develop --command bash tools/registry-tls-e2e.sh` passed on native Linux
  x86-64 and ARM64. A disposable HTTPS Distribution registry required generated
  credentials. Its generated CA was trusted only within the test process tree;
  TLS verification remained enabled. The revision and two explicit tags shared
  a manifest digest, exactly those three tags existed, and the pulled image ran.
- `nix develop --command bash tools/remote-builder-e2e.sh` passed on native Linux
  x86-64. A rootless container hosted SSH and an independent Nix store, without
  mounting the host store. A unique image label forced a fresh derivation.
  Coldeer constructed the image with local builds disabled (`max-jobs = 0`) and
  an explicitly configured SSH Linux builder. The result existed in that store,
  was returned to the host, and its streamed image loaded and ran successfully.

The [external GHCR run](https://github.com/sarifb/coldeer/actions/runs/41081600248)
passed at `fa305a882d1fdd6916d073b7f554d460e31d12d3` on native Linux x86-64:
`nix develop --command bash tools/registry-ghcr-e2e.sh <disposable-ghcr-repository>`
authenticated with the temporary workflow token and used normal public TLS
trust. Streaming push, equal digests for the revision and two explicit tags,
exactly those tags with no implicit `latest`, and authenticated pull/run all
passed. The workflow's separate cleanup step successfully deleted its unique
test package. No persistent registry credentials were needed or logged.

CI exposed an Intel Darwin dependency evaluation issue: the graph tool's
upstream igraph tests pull in optional plotting dependencies whose Arrow package
is marked broken on that platform. The shell disables igraph's upstream
build-time tests only on Intel Darwin; Coldeer's real graph-partitioner tests
remain enabled. A Linux subprocess fixture race also produced `ETXTBSY` once;
the three unit tests that write and execute scripts now share a mutex so their
forked children cannot temporarily inherit one another's writable script fds.

Remaining limits: the SSH builder is a separate container on the same Linux
runner, rather than a separate physical host or a Darwin-to-Linux setup.
Rootless evidence applies to the configured Ubuntu runners. Concurrent Lima
validation runs remain untested. These experiments do not claim exhaustive
behavior-by-behavior parity with Snowydeer.

## Historical checkout results

Last run: 2026-10-30. The working tree was based on Git revision
`f79f6cc762d0a2d5175d50993224ded123f80e20`, with the Linux validation changes
present but uncommitted. Image revision labels and pushed revision tags identify
that HEAD; they do not claim that uncommitted files belong to the commit.

### Historical platforms

| Platform     | Compilation and execution                                        | Containers                                         |
| ------------ | ---------------------------------------------------------------- | -------------------------------------------------- |
| macOS ARM64  | Cargo, native Buck2, package and BXL imports                     | Linux VM managed from this host                    |
| Linux ARM64  | Full validation suite in Lima VZ on this Mac                     | Build, save, load, run, and loopback-registry push |
| macOS x86-64 | Flake outputs evaluated only                                     | Not run                                            |
| Linux x86-64 | Flake outputs evaluated; Alpine archive prefetched and validated | No execution or image construction                 |

The Linux guest is Ubuntu 24.04, kernel `6.8.0-147-generic`, with 4 virtual CPUs
and 6 GiB RAM. It uses Nix 2.34.7, Rust 1.95.0, and the repository's locked
Nixpkgs and Buck2. Host filesystems are not mounted into the guest. The VM disk
lives outside the repository so Buck2's filesystem crawler does not hash it.

### Historical Linux ARM64 results

`nix develop --command tools/linux-vm.sh test` passed:

- `cargo test --locked --workspace`: 27 tests (14 library and 13 process tests).
  The demo also compiled. Three tool-dependent tests are ignored by default.
- Both explicitly enabled layer graph tests, using the actual Nixpkgs
  `flatten_references_graph`, passed. These check stable dependency partitions,
  complete nonduplicated coverage, and the layer budget.
- The explicitly enabled real Nix-daemon test passed with CA derivations disabled
  and an untrusted client. It imports a dependency and an object referring to it
  into an isolated temporary store, verifies bytes, and removes that store.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` passed.
- Native `buck2 test //:unit-tests` passed: 14 tests, one tool-dependent test
  ignored. Buck2 builds Rust directly, without a Cargo action.
- `buck2 build //demo:alpine` validated the active architecture's checked-in pin
  through the public base-image rule.
- `COLDEER_TEST_CONTAINERS=1 tools/e2e.sh` passed. This covers native and imported
  hello-world execution; provider-based Rust/C++ dependency discovery; arbitrary
  target BXL imports with both backends; exact import bytes and references;
  exclusion of unused candidates; repeated-import address stability; matching
  direct/derivation addresses; content-address metadata; and direct imports with
  CA derivations disabled on the client.
- C++ fixtures executed successfully. `ldd` confirmed automatic static linkage
  of the Buck-built greeting library, while explicit shared linkage through
  target attributes and platform constraints was preserved.
- The demo image was built on Linux, saved to a Docker archive, loaded into
  Podman, and executed successfully.
- `tools/container-e2e.sh` built a GC-rooted image-stream executable and piped
  its archive into Podman. The hello-world command ran successfully as UID/GID 65534. Runtime checks verified the working directory, `GIT_SHA1`, custom and
  locale environment, writable sticky `/tmp`, and tini as PID 1. Image inspection
  verified OCI revision/version labels and TCP/UDP exposed ports.
- A container using the checked-in Alpine digest/archive pin built and ran both
  the Buck-built hello executable and Alpine's BusyBox.
- Two imported application roots differed only by a version file. Their actual
  archives had different application/rootfs layers and two byte-identical shared
  layers. Every layer without the changed root or its symlinks was retained.
- A temporary Distribution registry listened on loopback in the guest. Real
  streaming pushes succeeded. The full Git revision and two explicitly requested
  tags resolved to the same manifest digest; an implicit `latest` tag was absent.
  Podman pulled the pushed image and ran it successfully.
- Both changed application images were pushed to that registry. Two compressed
  layer digests were identical across the images, matching the shared archive
  layers; the other digests changed. This checks actual registry layer reuse,
  beyond the graph partitioner's output.

The temporary registry and its storage are removed after the check. No external
registry was pushed to. Podman's unsigned-image policy and the loopback registry's
HTTP exception are confined to the temporary test configuration. The guest's
Nix daemon enables CA derivations for the retained derivation-backend checks;
the isolated daemon test deliberately disables them.

Linux validation found and fixed a missing Nix C++ runtime search path. The
native toolchain now embeds the pinned GCC runtime library directory on Linux.
It also found a Linux-specific deadlock in the isolated test's bidirectional
proxy: explicit bounded reads/writes replace `io::copy`'s optimized splice path.
The production daemon client already uses explicit streaming reads/writes.

### Historical macOS and earlier checks

On macOS ARM64, Cargo tests, formatting, native Buck2 builds, and the real
package/BXL/import E2E checks passed. The C++ fixtures were inspected with
`otool -L`. The isolated real-daemon test also passed on this host.

Earlier validation additionally checked:

- Real Alpine 3.23 update/lock/validate for ARM64 and x86-64 archive pins. Both
  architecture hashes validated, and the manifest-list digest was shared.
- Plain Buck2 builds using pinned immutable toolchain paths, including a fresh
  isolated Buck daemon with no Rust on PATH. An October 2026 Buck2 installation
  also built the CLI and package and ran its native tests.
- An earlier Nix-packaged CLI build and its tests on macOS ARM64.
- Schema rejection, metadata generation, revision/explicit tags, architecture
  selection, JSON prefetch, lock contention and atomic replacement, rollback
  after a failed architecture, malformed pins, and child-process failures using
  isolated fake tools. Real pipe tests check binary bytes, failed producers,
  stopping failed uploads, and digest-based retagging.

### Historical limits at that revision

No executables were run on macOS x86-64 or Linux x86-64. Evaluation of all four
flake platform outputs is not execution coverage. Lima uses the Mac's native
architecture; an ARM64 VM does not validate x86-64 compilation or execution.

External authenticated registries, TLS authentication, remote Nix builders,
rootless Podman, and concurrent VM validation runs were not exercised. The
registry integration check uses a disposable, unauthenticated loopback service.
