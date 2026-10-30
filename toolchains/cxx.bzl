"""Attach Nix runtime candidates to the C++ toolchain used by Rust linking."""
load("@root//rules:providers.bzl", "ColdeerNixCandidates")
load("@nix_toolchain//:defs.bzl", "CXX_NIX_PATHS")

def _impl(ctx):
    return ctx.attrs.actual.providers + [
        ColdeerNixCandidates(paths = CXX_NIX_PATHS, path_files = []),
    ]

coldeer_cxx_toolchain = rule(
    impl = _impl,
    is_toolchain_rule = True,
    attrs = {"actual": attrs.toolchain_dep()},
)
