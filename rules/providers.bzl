"""Coldeer's small, frontend-independent Nix dependency protocol."""
ColdeerNixInfo = provider(fields = {
    "store_path_file": Artifact,
})
ColdeerNixCandidates = provider(fields = {
    "paths": list,
    "path_files": list,
})
ColdeerBaseImageInfo = provider(fields = {
    "lock_file": Artifact,
})

def candidates(deps):
    paths = []
    files = []
    for dep in deps:
        if ColdeerNixCandidates in dep:
            paths.extend(dep[ColdeerNixCandidates].paths)
            files.extend(dep[ColdeerNixCandidates].path_files)
        if ColdeerNixInfo in dep:
            files.append(dep[ColdeerNixInfo].store_path_file)
    return (dedupe(paths), dedupe(files))

def declare_import(actions, tool, artifact, name, paths, files, suffix = "", backend = "direct"):
    output = actions.declare_output("store_path" + suffix + ".txt")
    spec = actions.write_json("import" + suffix + ".json", {
        "schema_version": 1,
        "name": name,
        "artifact": artifact,
        "candidate_paths": paths,
        "candidate_path_files": files,
        "import_backend": backend,
    }, with_inputs = True)
    actions.run(
        cmd_args(tool, "import", spec, output.as_output()),
        category = "coldeer_import",
        identifier = suffix or name,
        local_only = True,
        allow_cache_upload = False,
    )
    return output
