"""Pins live in JSON sources, never rewritten BUCK files."""
load(":providers.bzl", "ColdeerBaseImageInfo")

def _base_impl(ctx):
    spec = ctx.actions.write_json("base-image.json", {
        "schema_version": 1,
        "image_name": ctx.attrs.image_name,
        "follows_tag": ctx.attrs.follows_tag,
        "lock_file": ctx.attrs.lock_file,
        "system": ctx.attrs.system,
        "systems": ctx.attrs.systems,
    }, with_inputs = True)
    validated = ctx.actions.declare_output("validated.txt")
    ctx.actions.run(
        cmd_args(ctx.attrs._cli[RunInfo], "base-image", spec, "validate", "--out", validated.as_output()),
        category = "coldeer_base_image_validate",
        local_only = True,
        allow_cache_upload = False,
    )
    return [
        DefaultInfo(default_output = validated),
        RunInfo(args = cmd_args(ctx.attrs._cli[RunInfo], "base-image", spec)),
        ColdeerBaseImageInfo(lock_file = ctx.attrs.lock_file),
    ]

coldeer_base_image = rule(impl = _base_impl, attrs = {
    "image_name": attrs.string(),
    "follows_tag": attrs.string(default = "latest"),
    "lock_file": attrs.source(),
    "system": attrs.enum(["aarch64-linux", "x86_64-linux"], default = select({"@prelude//cpu:arm64": "aarch64-linux", "@prelude//cpu:x86_64": "x86_64-linux"})),
    "systems": attrs.list(attrs.string(), default = ["aarch64-linux", "x86_64-linux"]),
    "_cli": attrs.exec_dep(default = "//:coldeer", providers = [RunInfo]),
})
