"""Container RunInfo passes a versioned plan directly to the Rust executable."""
load(":providers.bzl", "ColdeerBaseImageInfo", "ColdeerNixInfo")

def _container_impl(ctx):
    metadata = dict(ctx.attrs.labels)
    metadata.update({
        "org.opencontainers.image.description": ctx.attrs.description,
        "org.opencontainers.image.title": str(ctx.label.raw_target()),
        "org.opencontainers.image.source": str(ctx.label),
    })
    plan = {
        "schema_version": 1,
        "name": ctx.attrs.image_name or ctx.label.name,
        "system": ctx.attrs.system,
        "cmd": ctx.attrs.cmd,
        "env": ctx.attrs.env,
        "ports": ctx.attrs.ports,
        "metadata": metadata,
        "content_path_files": [d[ColdeerNixInfo].store_path_file for d in ctx.attrs.contents],
        "main_path_files": [d[ColdeerNixInfo].store_path_file for d in ctx.attrs.main_contents],
        "working_dir": ctx.attrs.working_dir,
        "user": ctx.attrs.user,
    }
    if ctx.attrs.base_image != None:
        plan["base_lock_file"] = ctx.attrs.base_image[ColdeerBaseImageInfo].lock_file
    output = ctx.actions.declare_output("container.json")
    plan_args = ctx.actions.write_json(output, plan, with_inputs = True)
    return [DefaultInfo(default_output = output), RunInfo(args = cmd_args(ctx.attrs._cli[RunInfo], "container", plan_args))]

coldeer_container = rule(impl = _container_impl, attrs = {
    "description": attrs.string(),
    "cmd": attrs.list(attrs.string()),
    "image_name": attrs.string(default = ""),
    "system": attrs.enum(["aarch64-linux", "x86_64-linux"], default = select({"@prelude//cpu:arm64": "aarch64-linux", "@prelude//cpu:x86_64": "x86_64-linux"})),
    "env": attrs.dict(attrs.string(), attrs.string(), default = {}),
    "ports": attrs.list(attrs.string(), default = []),
    "labels": attrs.dict(attrs.string(), attrs.string(), default = {}),
    "contents": attrs.list(attrs.dep(providers = [ColdeerNixInfo]), default = []),
    "main_contents": attrs.list(attrs.dep(providers = [ColdeerNixInfo]), default = []),
    "base_image": attrs.option(attrs.dep(providers = [ColdeerBaseImageInfo]), default = None),
    "working_dir": attrs.string(default = "/"),
    "user": attrs.string(default = ""),
    "_cli": attrs.exec_dep(default = "//:coldeer", providers = [RunInfo]),
})
