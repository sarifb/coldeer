# CLI contracts, schema version 1

Every JSON document crossing the Buck2/Rust boundary has integer
`schema_version: 1`. Unsupported versions, duplicate keys, malformed JSON,
invalid field types, and nesting beyond 128 levels fail with a nonzero exit.
Unknown fields are retained or ignored to permit compatible additions. Required
fields listed below must be present. Diagnostics go to stderr; save-to-stdout
writes only archive bytes. External tools are invoked without a shell.

## Import

`coldeer import SPEC.json OUTPUT_PATH_FILE`

```json
{
  "schema_version": 1,
  "name": "application",
  "artifact": "buck-out/path/to/file-or-directory",
  "import_backend": "direct",
  "candidate_paths": ["/nix/store/00000000000000000000000000000000-dependency"],
  "candidate_path_files": ["buck-out/another-package/store_path.txt"]
}
```

`name` and `artifact` are required. Candidate arrays default to empty. A path
file contains exactly one store root, with optional surrounding whitespace.
The result is the same single-line format for compatibility with Nix tooling.
The versioned JSON spec defines that leaf format; it is not arbitrary JSON.
The output is atomically replaced after successful import and root creation.
Missing optional candidate outputs are ignored; Nix closure failures are fatal.

`import_backend` defaults to `direct`, a streaming Rust client for a local Nix
daemon supporting protocol 1.25+. It does not build a derivation or require
`ca-derivations`. `derivation` retains the previous copy-through-CA-derivation
backend, including its daemon feature requirement for reference-bearing inputs.
There is no implicit fallback. Direct imports support the default daemon,
`NIX_DAEMON_SOCKET_PATH`, and Unix socket `NIX_REMOTE` URIs; other store transports
can use the compatibility backend. Both produce the same address for identical
names, NAR bytes, and references, and keep dependencies at their original paths.

## Container

`coldeer container PLAN.json plan|build|save|push [ARGS]`

```json
{
  "schema_version": 1,
  "name": "application",
  "system": "aarch64-linux",
  "cmd": ["/bin/application"],
  "contents": [],
  "main_contents": [],
  "content_path_files": [],
  "main_path_files": [],
  "env": { "PORT": "8080" },
  "ports": ["8080/tcp"],
  "metadata": {
    "org.opencontainers.image.description": "An application",
    "org.opencontainers.image.title": "//app:container",
    "org.opencontainers.image.source": "//app:container (configuration)"
  },
  "working_dir": "/",
  "user": "1000:1000",
  "base_lock_file": "images/base.lock.json"
}
```

`schema_version`, `name`, `system`, nonempty `cmd`, and the three nonempty OCI
metadata fields are required. Arrays, environment, user, working directory,
and base lock are optional. `contents` and `main_contents` contain store roots;
corresponding path-file arrays are merged, sorted, and deduplicated. Metadata
values and environment values must be strings. Ports use nonzero u16 numbers
and `/tcp` or `/udp`. Plans reject non-Linux systems. `plan` prints the resolved,
versioned JSON with Git metadata and resolved base pins and performs no image
build. `build GC_ROOT` prints the built streaming-script store path.
`save [FILE]` streams to stdout or atomically replaces FILE.
`push REGISTRY/REPOSITORY [--tag TAG ...]` always includes the Git revision.
It streams once to that revision tag, reads the registry digest, and creates
additional tags from that digest with digest preservation. Coldeer writes no
temporary image archive. A producer failure, upload failure, or invalid returned
digest prevents additional tagging. `GIT_SHA1` always matches the generated
revision metadata. Resolved plans also contain `layering_pipeline`, generated
by Rust for Nix's graph partitioner.

## Base-image specification

`coldeer base-image SPEC.json validate|update|lock [--out FILE]`

```json
{
  "schema_version": 1,
  "image_name": "docker.io/library/alpine",
  "follows_tag": "3.23",
  "lock_file": "images/alpine.lock.json",
  "system": "aarch64-linux",
  "systems": ["aarch64-linux", "x86_64-linux"]
}
```

Paths are relative to the working directory (Buck2 uses the project root).
`update` requires the image name, follows tag, and systems. `lock` requires an
existing digest. `validate` requires a complete active-system pin and prints its
pulled archive store path, or writes it to `--out`. Update/lock do not build the
base target's validation output. No BUCK source is edited.

Lock format:

```json
{
  "schema_version": 1,
  "image_name": "docker.io/library/alpine",
  "image_digest": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "nar_hashes": {
    "aarch64-linux": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
    "x86_64-linux": "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
  }
}
```

The example hashes illustrate syntax and are not usable image pins. Empty pins
are accepted only as bootstrap input to `update`. Image names in a spec and its
existing lock must agree. All architecture prefetches must succeed before any
lock replacement. Guard files coordinate Coldeer processes; they are not a
substitute for source-control conflict resolution.

## Tools and failures

Runtime executables: `nix`, `nix-store` for GC roots, `git` for metadata, `skopeo` for tag resolution and
registry pushes, and `nix-prefetch-docker` for lock maintenance. Buck2 compilation
uses `rustc`, the C++ linker toolchain, and the bundled prelude's helpers. Nix
expressions are embedded in the Rust binary, so a compiled CLI can run away from
the source tree. Child failures, I/O failures, invalid pins, and unavailable
builders return exit status 1. Version/help return 0. No network operation is
retried as an implicit registry push or a tag change.
