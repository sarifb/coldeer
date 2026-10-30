"""Build a file tree natively, with an on-demand content-addressed Nix import."""
load(":providers.bzl", "ColdeerNixCandidates", "ColdeerNixInfo", "candidates", "declare_import")

def _tree_impl(ctx):
    files = dict(ctx.attrs.extra_files)
    for dep in ctx.attrs.binaries:
        outputs = dep[DefaultInfo].default_outputs
        if len(outputs) != 1:
            fail("each binary must have exactly one default output")
        artifact = outputs[0]
        dest = "bin/" + artifact.basename
        if dest in files:
            fail("duplicate package destination: " + dest)
        files[dest] = artifact
    for dest in files:
        if dest.startswith("/") or ".." in dest.split("/") or dest == "":
            fail("package destinations must be relative and cannot contain '..'")
    tree = ctx.actions.copied_dir("package", files)
    return [DefaultInfo(default_output = tree)]

_tree = rule(impl = _tree_impl, attrs = {
    "binaries": attrs.list(attrs.transition_dep(cfg = "//rules:package_linking"), default = []),
    "extra_files": attrs.dict(attrs.string(), attrs.source(), default = {}),
})

def _package_impl(ctx):
    paths, files = candidates(ctx.attrs.closure)
    paths.extend(ctx.attrs.nix_paths)
    tree = ctx.attrs.artifact[DefaultInfo].default_outputs[0]
    imported = declare_import(ctx.actions, ctx.attrs._cli[RunInfo], tree, ctx.label.name, dedupe(paths), files, backend = ctx.attrs.import_backend)
    return [
        DefaultInfo(default_output = tree, sub_targets = {"store_path": [DefaultInfo(default_output = imported)]}),
        ColdeerNixInfo(store_path_file = imported),
        ColdeerNixCandidates(paths = dedupe(paths), path_files = files),
    ]

_package = rule(impl = _package_impl, attrs = {
    "artifact": attrs.dep(),
    "closure": attrs.query(),
    "nix_paths": attrs.list(attrs.string(), default = []),
    "import_backend": attrs.enum(["direct", "derivation"], default = "direct"),
    "_cli": attrs.exec_dep(default = "//:coldeer", providers = [RunInfo]),
})

def coldeer_package(name, binaries = [], extra_files = {}, nix_paths = [], import_backend = "direct", **kwargs):
    artifact = name + "__tree"
    _tree(name = artifact, binaries = binaries, extra_files = extra_files, visibility = [":" + name])
    _package(name = name, artifact = ":" + artifact, closure = "deps(:" + artifact + ")", nix_paths = nix_paths, import_backend = import_backend, **kwargs)

def _nix_dependency_impl(ctx):
    path_file = ctx.actions.write("store_path.txt", ctx.attrs.path + "\n")
    return [DefaultInfo(default_output = path_file), ColdeerNixInfo(store_path_file = path_file), ColdeerNixCandidates(paths = [ctx.attrs.path], path_files = [])]

# Adapter for packages provided by another Nix integration. This deliberately
# does not couple consumers to one repository's Nix toolchain providers.
coldeer_nix_dependency = rule(impl = _nix_dependency_impl, attrs = {"path": attrs.string()})
