load("@prelude//:rules.bzl", "rust_binary", "rust_library", "rust_test")

rust_library(
    name = "lib",
    crate = "coldeer",
    crate_root = "src/lib.rs",
    srcs = glob(["src/**/*.rs", "nix/*.nix"]) + ["flake.lock"],
    edition = "2021",
    visibility = ["PUBLIC"],
)

rust_binary(
    name = "coldeer",
    crate_root = "src/main.rs",
    srcs = ["src/main.rs"],
    deps = [":lib"],
    edition = "2021",
    visibility = ["PUBLIC"],
)

rust_test(
    name = "unit-tests",
    crate = "coldeer",
    crate_root = "src/lib.rs",
    srcs = glob(["src/**/*.rs", "nix/*.nix"]) + ["flake.lock"],
    edition = "2021",
)
