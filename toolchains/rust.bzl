"""Native Buck2 Rust compilation using the immutable Nix toolchain cell."""
load("@prelude//rust:rust_toolchain.bzl", "PanicRuntime", "RustToolchainInfo")
load("@root//rules:providers.bzl", "ColdeerNixCandidates")
load("@nix_toolchain//:defs.bzl", "CLIPPY", "RUSTC", "RUSTDOC", "RUST_NIX_PATHS")

def _impl(ctx):
    return [
        DefaultInfo(),
        RustToolchainInfo(
            compiler = RunInfo(args = [RUSTC]),
            rustdoc = RunInfo(args = [RUSTDOC]),
            clippy_driver = RunInfo(args = [CLIPPY]),
            default_edition = "2021",
            nightly_features = False,
            panic_runtime = PanicRuntime("unwind"),
            rustc_target_triple = ctx.attrs.triple,
        ),
        ColdeerNixCandidates(paths = RUST_NIX_PATHS, path_files = []),
    ]

coldeer_rust_toolchain = rule(impl = _impl, is_toolchain_rule = True, attrs = {
    "triple": attrs.string(default = select({
        "prelude//os:macos": select({"prelude//cpu:arm64": "aarch64-apple-darwin", "prelude//cpu:x86_64": "x86_64-apple-darwin"}),
        "prelude//os:linux": select({"prelude//cpu:arm64": "aarch64-unknown-linux-gnu", "prelude//cpu:x86_64": "x86_64-unknown-linux-gnu"}),
    })),
})
