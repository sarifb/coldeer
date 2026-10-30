//! Versioned build plans and process orchestration. Child diagnostics always go to stderr.
use crate::{
    json::{self, Value as V},
    scan_references, Result,
};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn read_json(path: impl AsRef<Path>) -> Result<V> {
    json::parse(&fs::read_to_string(path)?)
}

pub fn versioned(fields: impl IntoIterator<Item = (&'static str, V)>) -> V {
    let mut v = V::object(fields);
    v.set("schema_version", V::Number("1".into())).unwrap();
    v
}

pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let tmp = parent.join(format!(
        ".coldeer-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}

struct Temp(PathBuf);

impl Temp {
    fn new() -> Result<Self> {
        let p = std::env::temp_dir().join(format!(
            "coldeer-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p)?;
        Ok(Self(p))
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn output(cmd: &mut Command) -> Result<String> {
    let result = cmd.stderr(Stdio::inherit()).output()?;
    if !result.status.success() {
        return Err(format!(
            "{} failed ({})",
            cmd.get_program().to_string_lossy(),
            result.status
        )
        .into());
    }
    Ok(String::from_utf8(result.stdout)?.trim().into())
}

fn status(cmd: &mut Command) -> Result<()> {
    let s = cmd.status()?;
    if !s.success() {
        return Err(format!("{} failed ({s})", cmd.get_program().to_string_lossy()).into());
    }
    Ok(())
}

fn nix() -> Command {
    let mut c = Command::new("nix");
    c.args(["--extra-experimental-features", "nix-command flakes"]);
    c
}

pub fn store_path(s: &str) -> Result<()> {
    scan_references(&b""[..], &[s.to_string()])?;
    if s.len() < 45
        || !s[44..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+-._?=".contains(&b))
    {
        return Err("expected a store root path".into());
    }
    Ok(())
}

fn text<'a>(v: &'a V, key: &str) -> Result<&'a str> {
    v.get(key)?.str()
}

fn strings(v: &V, key: &str) -> Result<Vec<String>> {
    v.optional(key).map(V::strings).unwrap_or(Ok(vec![]))
}

fn path_file(path: &str) -> Result<String> {
    let s = fs::read_to_string(path)?;
    let s = s.trim();
    store_path(s)?;
    Ok(s.into())
}

fn nix_build(expr: &str, plan: &V, root: &Path, ca: bool) -> Result<String> {
    let t = Temp::new()?;
    let expression = t.file("build.nix");
    fs::write(&expression, expr)?;
    let pins = match std::env::var("COLDEER_NIXPKGS") {
        Ok(path) => path,
        Err(_) => {
            let lock = json::parse(include_str!("../flake.lock"))?;
            let pin = lock.get("nodes")?.get("nixpkgs")?.get("locked")?;
            let hash = text(pin, "narHash")?
                .replace('+', "%2B")
                .replace('/', "%2F")
                .replace('=', "%3D");
            format!("github:NixOS/nixpkgs/{}?narHash={hash}", text(pin, "rev")?)
        }
    };
    let mut c = nix();
    if ca {
        c.args(["--extra-experimental-features", "ca-derivations"]);
    }
    c.args(["build", "--impure", "--json", "--file"])
        .arg(expression)
        .args([
            "--argstr",
            "nixpkgs",
            &pins,
            "--argstr",
            "planJSON",
            &json::encode(plan),
            "--out-link",
        ])
        .arg(root);
    let result = json::parse(&output(&mut c)?)?;
    let path = text(
        result
            .list()?
            .first()
            .ok_or("empty Nix build result")?
            .get("outputs")?,
        "out",
    )?
    .to_string();
    store_path(&path)?;
    Ok(path)
}

/// Expand Buck-level candidates through Nix, then scan the exact NAR bytes.
pub fn import(spec: &Path, out: &Path) -> Result<()> {
    let v = read_json(spec)?;
    v.version()?;
    let backend = v
        .optional("import_backend")
        .map(V::str)
        .transpose()?
        .unwrap_or("direct");
    if !matches!(backend, "direct" | "derivation") {
        return Err("import_backend must be direct or derivation".into());
    }
    let name = text(&v, "name")?;
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+-._?=".contains(&b))
        || name.starts_with('.')
    {
        return Err("invalid store name".into());
    }
    let artifact = fs::canonicalize(text(&v, "artifact")?)?;
    let mut candidates: BTreeSet<String> = strings(&v, "candidate_paths")?.into_iter().collect();
    for p in strings(&v, "candidate_path_files")? {
        candidates.insert(path_file(&p)?);
    }
    for p in &candidates {
        store_path(p)?;
    }
    // Absent optional outputs cannot have been observed by the artifact.
    candidates.retain(|p| Path::new(p).symlink_metadata().is_ok());
    let t = Temp::new()?;
    let nar = t.file("artifact.nar");
    status(
        nix()
            .args(["nar", "dump-path"])
            .arg(&artifact)
            .stdout(File::create(&nar)?),
    )?;
    let closure = if candidates.is_empty() {
        vec![]
    } else {
        let mut c = nix();
        c.args(["path-info", "--recursive"]);
        c.args(candidates);
        output(&mut c)?.lines().map(str::to_owned).collect()
    };
    let references = scan_references(File::open(&nar)?, &closure)?;
    let root = std::path::absolute(format!("{}.gcroot", out.display()))?;
    if backend == "direct" {
        let path = crate::daemon::import(&nar, name, &references, &root)?;
        return atomic_write(out, format!("{path}\n").as_bytes());
    }
    // Nix's public add-path API directly supports the reference-free CA case.
    // This avoids a builder and the daemon's CA-derivation feature entirely.
    if references.is_empty() {
        let path = output(
            nix()
                .args(["store", "add-path", "--name", name])
                .arg(&artifact),
        )?;
        store_path(&path)?;
        status(
            Command::new("nix-store")
                .arg("--add-root")
                .arg(&root)
                .args(["--indirect", "--realise", &path])
                .stdout(Stdio::null()),
        )?;
        return atomic_write(out, format!("{path}\n").as_bytes());
    }
    let plan = versioned([
        ("name", name.into()),
        ("artifact", artifact.to_string_lossy().to_string().into()),
        ("references", V::array(references)),
    ]);
    let path = nix_build(include_str!("../nix/import.nix"), &plan, &root, true)?;
    atomic_write(out, format!("{path}\n").as_bytes())?;
    Ok(())
}

fn image_system(s: &str) -> Result<&'static str> {
    match s {
        "aarch64-linux" => Ok("arm64"),
        "x86_64-linux" => Ok("amd64"),
        _ => Err(
            "container system must be aarch64-linux or x86_64-linux; configure a Linux Nix builder"
                .into(),
        ),
    }
}

fn digest(s: &str) -> Result<()> {
    if s.len() != 71 || !s.starts_with("sha256:") || !s[7..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("invalid sha256 image digest".into());
    }
    Ok(())
}

fn validate_repository(s: &str) -> Result<()> {
    if s.is_empty()
        || s.starts_with(['-', '/', '.'])
        || s.ends_with('/')
        || s.contains("//")
        || s.contains('@')
        || s.rsplit('/').next().unwrap_or("").contains(':')
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"./:_-[]".contains(&b))
    {
        return Err("expected an untagged image repository".into());
    }
    Ok(())
}

fn image_tag(tag: &str) -> Result<()> {
    if tag.is_empty()
        || tag.len() > 128
        || !tag
            .bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_alphanumeric() || b == b'_' || (i > 0 && b".-".contains(&b)))
    {
        return Err(format!("invalid image tag {tag:?}").into());
    }
    Ok(())
}

pub fn validate_lock(v: &V) -> Result<()> {
    v.version()?;
    digest(text(v, "image_digest")?)?;
    validate_repository(text(v, "image_name")?)?;
    let hashes = v.get("nar_hashes")?.map()?;
    if hashes.is_empty() {
        return Err("base image has no system pins".into());
    }
    for (s, h) in hashes {
        image_system(s)?;
        let h = h.str()?;
        if h.len() != 51
            || !h.starts_with("sha256-")
            || !h.ends_with('=')
            || !h.as_bytes()[7..50]
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || b"+/".contains(b))
            || !b"AEIMQUYcgkosw048".contains(&h.as_bytes()[49])
        {
            return Err("invalid SHA256 SRI image hash".into());
        }
    }
    Ok(())
}

fn prefetch(image: &str, digest: &str, system: &str) -> Result<String> {
    let arch = image_system(system)?;
    let v = json::parse(&output(Command::new("nix-prefetch-docker").args([
        "--json",
        "--image-name",
        image,
        "--image-digest",
        digest,
        "--os",
        "linux",
        "--arch",
        arch,
        "--final-image-name",
        image,
        "--final-image-tag",
        "pinned",
    ]))?)?;
    if let Some(h) = v.optional("hash") {
        return Ok(h.str()?.into());
    }
    let h = text(&v, "sha256")?;
    output(nix().args(["hash", "convert", "--hash-algo", "sha256", "--to", "sri", h]))
}

struct Guard(PathBuf);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// The lock is only replaced after every architecture succeeds. Guard prevents lost updates.
pub fn base_image(spec: &Path, action: &str, out: Option<&Path>) -> Result<()> {
    let s = read_json(spec)?;
    s.version()?;
    let lock_path = PathBuf::from(text(&s, "lock_file")?);
    if out.is_some() && action != "validate" {
        return Err("--out is only supported for base-image validate".into());
    }
    if action == "validate" {
        let lock = read_json(&lock_path)?;
        validate_lock(&lock)?;
        if text(&s, "image_name")? != text(&lock, "image_name")? {
            return Err("lock image_name disagrees with spec".into());
        }
        let system = text(&s, "system")?;
        image_system(system)?;
        let hash = text(lock.get("nar_hashes")?, system)?;
        let plan = versioned([
            ("system", system.into()),
            (
                "base_image",
                V::object([
                    ("image_name", lock.get("image_name")?.clone()),
                    ("image_digest", lock.get("image_digest")?.clone()),
                    ("hash", hash.into()),
                ]),
            ),
        ]);
        let t = Temp::new()?;
        let path = nix_build(
            include_str!("../nix/base-image.nix"),
            &plan,
            &t.file("root"),
            false,
        )?;
        if let Some(out) = out {
            atomic_write(out, format!("{path}\n").as_bytes())?;
        } else {
            println!("{path}");
        }
        return Ok(());
    }
    if action != "update" && action != "lock" {
        return Err("base-image action must be validate, update, or lock".into());
    }
    let guard_path = lock_path.with_extension("json.guard");
    let _f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&guard_path)
        .map_err(|e| format!("cannot acquire {}: {e}", guard_path.display()))?;
    let _guard = Guard(guard_path);
    let old = if lock_path.exists() {
        Some(read_json(&lock_path)?)
    } else {
        None
    };
    let image = text(&s, "image_name")?;
    validate_repository(image)?;
    if let Some(ref old) = old {
        old.version()?;
        if text(old, "image_name")? != image {
            return Err("lock image_name disagrees with spec".into());
        }
    }
    let systems = strings(&s, "systems")?;
    if systems.is_empty() {
        return Err("systems cannot be empty".into());
    }
    for system in &systems {
        image_system(system)?;
    }
    let pinned = if action == "update" {
        let tag = text(&s, "follows_tag")?;
        if tag.is_empty() {
            return Err("update requires follows_tag".into());
        }
        image_tag(tag)?;
        output(Command::new("skopeo").args([
            "--override-os",
            "linux",
            "--override-arch",
            image_system(&systems[0])?,
            "inspect",
            "--format",
            "{{.Digest}}",
            &format!("docker://{image}:{tag}"),
        ]))?
    } else {
        text(
            old.as_ref()
                .ok_or("lock requires an existing image_digest")?,
            "image_digest",
        )?
        .into()
    };
    digest(&pinned)?;
    let mut hashes = std::collections::BTreeMap::new();
    for system in systems {
        hashes.insert(system.clone(), V::from(prefetch(image, &pinned, &system)?));
    }
    let lock = versioned([
        ("image_name", image.into()),
        ("image_digest", pinned.into()),
        ("nar_hashes", V::Object(hashes)),
    ]);
    validate_lock(&lock)?;
    atomic_write(&lock_path, format!("{}\n", json::encode(&lock)).as_bytes())?;
    Ok(())
}

pub fn revision() -> Result<String> {
    let result = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .output()?;
    if !result.status.success() {
        return Err("no Git HEAD; create a commit before pushing revision tags".into());
    }
    let r = String::from_utf8(result.stdout)?.trim().to_owned();
    if !matches!(r.len(), 40 | 64) || !r.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid Git revision".into());
    }
    Ok(r)
}

pub fn push_tags(revision: &str, tags: &[String]) -> Result<Vec<String>> {
    let mut result = vec![revision.to_owned()];
    for tag in tags {
        image_tag(tag)?;
        if !result.contains(tag) {
            result.push(tag.clone())
        }
    }
    Ok(result)
}

pub fn container_plan(mut v: V, revision: &str) -> Result<V> {
    v.version()?;
    image_system(text(&v, "system")?)?;
    let name = text(&v, "name")?;
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-._/".contains(&b))
    {
        return Err("invalid image name".into());
    }
    if strings(&v, "cmd")?.is_empty() {
        return Err("container cmd cannot be empty".into());
    }
    for port in strings(&v, "ports")? {
        let (n, proto) = port
            .split_once('/')
            .ok_or("port must use NUMBER/tcp or NUMBER/udp")?;
        if n.parse::<u16>().ok().filter(|n| *n > 0).is_none() || !matches!(proto, "tcp" | "udp") {
            return Err("invalid exposed port".into());
        }
    }
    let mut env = v
        .optional("env")
        .cloned()
        .unwrap_or_else(|| V::object(Vec::<(String, V)>::new()));
    for (k, value) in env.map()? {
        if k.is_empty() || k.contains(['=', '\0']) || value.str()?.contains('\0') {
            return Err("invalid environment entry".into());
        }
    }
    env.set(
        "LANG",
        env.optional("LANG")
            .cloned()
            .unwrap_or_else(|| "C.UTF-8".into()),
    )?;
    env.set("GIT_SHA1", revision.into())?;
    v.set("env", env)?;
    let mut labels = v
        .optional("metadata")
        .cloned()
        .unwrap_or_else(|| V::object(Vec::<(String, V)>::new()));
    for val in labels.map()?.values() {
        val.str()?;
    }
    for key in [
        "org.opencontainers.image.description",
        "org.opencontainers.image.title",
        "org.opencontainers.image.source",
    ] {
        if text(&labels, key)?.trim().is_empty() {
            return Err(format!("empty OCI metadata {key}").into());
        }
    }
    labels.set("org.opencontainers.image.revision", revision.into())?;
    labels.set("org.opencontainers.image.version", revision.into())?;
    v.set("metadata", labels)?;
    for (key, files_key) in [
        ("contents", "content_path_files"),
        ("main_contents", "main_path_files"),
    ] {
        let mut paths: BTreeSet<_> = strings(&v, key)?.into_iter().collect();
        for f in strings(&v, files_key)? {
            paths.insert(path_file(&f)?);
        }
        for p in &paths {
            store_path(p)?;
        }
        v.set(key, V::array(paths))?;
    }
    if v.get("main_contents")?.list()?.len() > 119 {
        return Err("containers support at most 119 application roots".into());
    }
    v.set(
        "layering_pipeline",
        crate::layering::pipeline(&strings(&v, "main_contents")?),
    )?;
    if let Some(file) = v.optional("base_lock_file") {
        let base = read_json(file.str()?)?;
        validate_lock(&base)?;
        let hash = base.get("nar_hashes")?.get(text(&v, "system")?)?.clone();
        let mut base = base;
        base.set("hash", hash)?;
        v.set("base_image", base)?;
    }
    Ok(v)
}

pub fn container(spec: &Path, action: &str, args: &[String]) -> Result<()> {
    let raw = read_json(spec)?;
    raw.version()?;
    // Revision tags must be real Git revisions. No implicit `latest` tag.
    let rev = if action == "push" {
        revision()?
    } else {
        revision().unwrap_or_else(|_| "uncommitted".into())
    };
    let plan = container_plan(raw, &rev)?;
    if action == "plan" {
        println!("{}", json::encode(&plan));
        return Ok(());
    }
    let t = Temp::new()?;
    let root = t.file("image");
    match action {
        "build" => {
            if args.len() != 1 {
                return Err("container build requires an output GC-root path".into());
            }
            let root = std::path::absolute(&args[0])?;
            let path = nix_build(include_str!("../nix/container.nix"), &plan, &root, false)?;
            println!("{path}");
        }
        "save" => {
            if args.len() > 1 {
                return Err("save accepts at most one output filename".into());
            }
            let script = nix_build(include_str!("../nix/container.nix"), &plan, &root, false)?;
            save_stream(Path::new(&script), args.first().map(Path::new))?;
        }
        "push" => {
            let repository = args.first().ok_or("push requires a registry/repository")?;
            validate_repository(repository)?;
            let mut tags = Vec::new();
            let mut rest = &args[1..];
            while !rest.is_empty() {
                if rest.len() < 2 || rest[0] != "--tag" {
                    return Err("push options: --tag TAG (repeatable)".into());
                }
                tags.push(rest[1].clone());
                rest = &rest[2..];
            }
            let tags = push_tags(&rev, &tags)?;
            let script = nix_build(include_str!("../nix/container.nix"), &plan, &root, false)?;
            push_stream(Path::new(&script), repository, &tags)?;
        }
        _ => return Err("container action must be plan, build, save, or push".into()),
    }
    Ok(())
}

/// Stream once, then create additional tags from the registry's immutable digest.
pub fn push_stream(script: &Path, repository: &str, tags: &[String]) -> Result<()> {
    push_stream_using(script, repository, tags, Path::new("skopeo"))
}

fn push_stream_using(
    script: &Path,
    repository: &str,
    tags: &[String],
    skopeo: &Path,
) -> Result<()> {
    validate_repository(repository)?;
    let first = tags.first().ok_or("push requires a revision tag")?;
    for tag in tags {
        image_tag(tag)?;
    }
    let temp = Temp::new()?;
    let digest_file = temp.file("digest");
    let mut producer = Command::new(script).stdout(Stdio::piped()).spawn()?;
    let upload = Command::new(skopeo)
        .args(["copy", "--insecure-policy", "--digestfile"])
        .arg(&digest_file)
        .arg("docker-archive:/dev/stdin")
        .arg(format!("docker://{repository}:{first}"))
        .stdin(producer.stdout.take().ok_or("missing archive stream")?)
        .spawn();
    let mut upload = match upload {
        Ok(child) => child,
        Err(e) => {
            let _ = producer.kill();
            let _ = producer.wait();
            return Err(e.into());
        }
    };
    let uploaded = upload.wait();
    if !uploaded.as_ref().is_ok_and(|s| s.success()) {
        let _ = producer.kill();
    }
    let produced = producer.wait()?;
    let uploaded = uploaded?;
    if !uploaded.success() {
        return Err(format!("image upload failed ({uploaded})").into());
    }
    if !produced.success() {
        return Err(format!("image streaming failed ({produced})").into());
    }
    let image_digest = fs::read_to_string(digest_file)?.trim().to_owned();
    digest(&image_digest)?;
    for tag in &tags[1..] {
        status(
            Command::new(skopeo)
                .args(["copy", "--insecure-policy", "--preserve-digests"])
                .arg(format!("docker://{repository}@{image_digest}"))
                .arg(format!("docker://{repository}:{tag}")),
        )?;
    }
    Ok(())
}

fn save_stream(script: &Path, destination: Option<&Path>) -> Result<()> {
    let Some(target) = destination else {
        return status(&mut Command::new(script));
    };
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let tmp = parent.join(format!(
        ".coldeer-archive-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        status(Command::new(script).stdout(file))?;
        File::open(&tmp)?.sync_all()?;
        fs::rename(&tmp, target)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executable(path: &Path, source: &str) {
        use std::os::unix::fs::PermissionsExt;
        fs::write(path, format!("#!/bin/sh\nset -eu\n{source}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn streaming_upload_and_digest_retagging() {
        let t = Temp::new().unwrap();
        let stream = t.file("stream");
        let skopeo = t.file("skopeo");
        executable(&stream, "printf 'archive\\000bytes\\377'");
        executable(
            &skopeo,
            &format!(
                r#"
dir=$(dirname "$0")
printf '%s\n' "$*" >> "$dir/log"
while [ "$#" -gt 0 ]; do
  if [ "$1" = --digestfile ]; then
    shift
    digestfile=$1
    cat > "$dir/received"
    printf 'sha256:{}\n' > "$digestfile"
    break
  fi
  shift
done
"#,
                "a".repeat(64)
            ),
        );
        push_stream_using(
            &stream,
            "registry.test/app",
            &["revision".into(), "release".into()],
            &skopeo,
        )
        .unwrap();
        assert_eq!(fs::read(t.file("received")).unwrap(), b"archive\0bytes\xff");
        let log = fs::read_to_string(t.file("log")).unwrap();
        let calls = log.lines().collect::<Vec<_>>();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].contains("docker-archive:/dev/stdin docker://registry.test/app:revision"));
        assert!(calls[1].contains(&format!(
            "docker://registry.test/app@sha256:{}",
            "a".repeat(64)
        )));
        assert!(calls[1].ends_with("docker://registry.test/app:release"));
        assert!(calls[1].contains("--preserve-digests"));
        executable(&stream, "printf partial; exit 19");
        fs::remove_file(t.file("log")).unwrap();
        assert!(push_stream_using(
            &stream,
            "registry.test/app",
            &["revision".into(), "release".into()],
            &skopeo
        )
        .is_err());
        assert_eq!(
            fs::read_to_string(t.file("log")).unwrap().lines().count(),
            1
        );
    }

    #[test]
    fn failed_upload_stops_the_producer() {
        let t = Temp::new().unwrap();
        let stream = t.file("stream");
        let skopeo = t.file("skopeo");
        executable(&stream, "while :; do printf 'data'; done");
        executable(&skopeo, "exit 23");
        assert!(
            push_stream_using(&stream, "registry.test/app", &["revision".into()], &skopeo).is_err()
        );
        assert!(push_stream_using(
            &stream,
            "registry.test/app",
            &["revision".into()],
            &t.file("missing")
        )
        .is_err());
    }

    #[test]
    fn stream_failure_does_not_publish_partial_archive() {
        use std::os::unix::fs::PermissionsExt;
        let t = Temp::new().unwrap();
        let script = t.file("stream");
        let archive = t.file("image.tar");
        fs::write(&archive, b"original").unwrap();
        fs::write(&script, b"#!/bin/sh\nprintf partial\nexit 19\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(save_stream(&script, Some(&archive)).is_err());
        assert_eq!(fs::read(&archive).unwrap(), b"original");
        assert_eq!(fs::read_dir(&t.0).unwrap().count(), 2);
        fs::write(&script, b"#!/bin/sh\nprintf complete\n").unwrap();
        save_stream(&script, Some(&archive)).unwrap();
        assert_eq!(fs::read(&archive).unwrap(), b"complete");
    }

    #[test]
    fn tags_are_explicit() {
        assert_eq!(
            push_tags("abc", &["release".into(), "abc".into()]).unwrap(),
            ["abc", "release"]
        );
        for bad in ["", "-bad", "x/y", "x:y"] {
            assert!(push_tags("abc", &[bad.into()]).is_err())
        }
    }

    #[test]
    fn lock_validation() {
        let v = versioned([
            ("image_name", "alpine".into()),
            ("image_digest", format!("sha256:{}", "a".repeat(64)).into()),
            (
                "nar_hashes",
                V::object([(
                    "aarch64-linux",
                    format!("sha256-{}=", "A".repeat(43)).into(),
                )]),
            ),
        ]);
        validate_lock(&v).unwrap();
        let mut bad = v;
        bad.set("schema_version", V::Number("2".into())).unwrap();
        assert!(validate_lock(&bad).is_err())
    }

    #[test]
    fn atomic_replace() {
        let t = Temp::new().unwrap();
        let p = t.file("lock.json");
        atomic_write(&p, b"old").unwrap();
        atomic_write(&p, b"new").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"new");
        assert_eq!(fs::read_dir(&t.0).unwrap().count(), 1)
    }

    #[test]
    fn strict_store_root() {
        for bad in [
            "/nix/store/00000000000000000000000000000000-foo/bin/x",
            "/nix/store/00000000000000000000000000000000-",
            "/tmp/x",
        ] {
            assert!(store_path(bad).is_err())
        }
    }
}
