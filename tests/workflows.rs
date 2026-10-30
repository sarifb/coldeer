//! Process-boundary tests use isolated fake tools; these never contact a registry.
use coldeer::{
    json::{self, Value as V},
    workflow::versioned,
};
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "coldeer-cli-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }

    fn path(&self, p: &str) -> PathBuf {
        self.0.join(p)
    }

    fn write(&self, p: &str, v: &V) {
        fs::write(self.path(p), json::encode(v)).unwrap()
    }

    fn script(&self, name: &str, body: &str) {
        let p = self.path(name);
        fs::write(&p, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
        fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap()
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_coldeer"))
            .args(args)
            .current_dir(&self.0)
            .env("PATH", format!("{}:/usr/bin:/bin", self.0.display()))
            .env("LOG", self.path("log"))
            .env("NIX_DAEMON_SOCKET_PATH", self.path("no-daemon.sock"))
            .env("NIX_REMOTE", "daemon")
            .output()
            .unwrap()
    }

    fn spec(&self) {
        self.write(
            "base.json",
            &versioned([
                ("lock_file", "image.lock.json".into()),
                ("image_name", "example.org/base".into()),
                ("follows_tag", "stable".into()),
                ("system", "aarch64-linux".into()),
                (
                    "systems",
                    V::array(["aarch64-linux".into(), "x86_64-linux".into()]),
                ),
            ]),
        );
    }

    fn fake_registry(&self) {
        self.script(
            "skopeo",
            &format!(
                "printf '%s\\n' \"$*\" >> \"$LOG\"\nprintf '%s\\n' 'sha256:{}'",
                "a".repeat(64)
            ),
        );
        self.script(
            "nix-prefetch-docker",
            &format!(
                "printf '%s\\n' \"$*\" >> \"$LOG\"\nprintf '%s\\n' '{{\"hash\":\"sha256-{}=\"}}'",
                "A".repeat(43)
            ),
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn success(o: &Output) {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr))
}

fn base() -> V {
    versioned([
        ("image_name", "example.org/base".into()),
        ("image_digest", format!("sha256:{}", "b".repeat(64)).into()),
        (
            "nar_hashes",
            V::object([
                (
                    "aarch64-linux",
                    format!("sha256-{}=", "E".repeat(43)).into(),
                ),
                ("x86_64-linux", format!("sha256-{}=", "E".repeat(43)).into()),
            ]),
        ),
    ])
}

fn plan() -> V {
    versioned([
        ("name", "demo".into()),
        ("system", "aarch64-linux".into()),
        ("cmd", V::array(["/bin/hello".into()])),
        (
            "metadata",
            V::object([
                ("org.opencontainers.image.title", "demo".into()),
                ("org.opencontainers.image.source", "//demo:container".into()),
                ("org.opencontainers.image.description", "hello".into()),
            ]),
        ),
    ])
}

#[test]
fn update_pins_every_architecture_atomically() {
    let f = Fixture::new();
    f.spec();
    f.fake_registry();
    f.write("image.lock.json", &base());
    let o = f.run(&["base-image", "base.json", "update"]);
    success(&o);
    let lock = coldeer::workflow::read_json(f.path("image.lock.json")).unwrap();
    assert_eq!(
        lock.get("image_digest").unwrap().str().unwrap(),
        format!("sha256:{}", "a".repeat(64))
    );
    assert_eq!(lock.get("nar_hashes").unwrap().map().unwrap().len(), 2);
    let log = fs::read_to_string(f.path("log")).unwrap();
    assert!(log.contains("--arch arm64"));
    assert!(log.contains("--arch amd64"));
    assert!(log.contains("docker://example.org/base:stable"));
    assert!(!f.path("image.lock.json.guard").exists())
}

#[test]
fn failed_second_arch_preserves_entire_lock() {
    let f = Fixture::new();
    f.spec();
    f.fake_registry();
    f.write("image.lock.json", &base());
    let before = fs::read(f.path("image.lock.json")).unwrap();
    f.script(
        "nix-prefetch-docker",
        &format!(
            "case \"$*\" in *amd64*) exit 19;; esac\nprintf '%s\\n' '{{\"hash\":\"sha256-{}=\"}}'",
            "A".repeat(43)
        ),
    );
    assert!(!f
        .run(&["base-image", "base.json", "update"])
        .status
        .success());
    assert_eq!(fs::read(f.path("image.lock.json")).unwrap(), before);
    assert!(!f.path("image.lock.json.guard").exists())
}

#[test]
fn lock_preserves_digest_and_does_not_resolve_tag() {
    let f = Fixture::new();
    f.spec();
    f.fake_registry();
    f.write("image.lock.json", &base());
    f.script("skopeo", "exit 97");
    success(&f.run(&["base-image", "base.json", "lock"]));
    let lock = coldeer::workflow::read_json(f.path("image.lock.json")).unwrap();
    assert_eq!(
        lock.get("image_digest").unwrap(),
        base().get("image_digest").unwrap()
    );
}

#[test]
fn concurrent_update_is_rejected() {
    let f = Fixture::new();
    f.spec();
    f.write("image.lock.json", &base());
    fs::write(f.path("image.lock.json.guard"), b"busy").unwrap();
    let before = fs::read(f.path("image.lock.json")).unwrap();
    assert!(!f
        .run(&["base-image", "base.json", "update"])
        .status
        .success());
    assert_eq!(fs::read(f.path("image.lock.json")).unwrap(), before);
    assert!(f.path("image.lock.json.guard").exists())
}

#[test]
fn container_plan_resolves_files_and_metadata() {
    let f = Fixture::new();
    f.script("git", &format!("echo {}", "c".repeat(40)));
    let mut p = plan();
    p.set("env", V::object([("GIT_SHA1", "user-value".into())]))
        .unwrap();
    let path = "/nix/store/00000000000000000000000000000000-hello";
    fs::write(f.path("store.txt"), format!("{path}\n")).unwrap();
    p.set(
        "main_path_files",
        V::array(["store.txt".into(), "store.txt".into()]),
    )
    .unwrap();
    f.write("plan.json", &p);
    let o = f.run(&["container", "plan.json", "plan"]);
    success(&o);
    let v = json::parse(std::str::from_utf8(&o.stdout).unwrap()).unwrap();
    assert_eq!(v.get("main_contents").unwrap().strings().unwrap(), [path]);
    assert_eq!(
        v.get("metadata")
            .unwrap()
            .get("org.opencontainers.image.revision")
            .unwrap()
            .str()
            .unwrap(),
        "c".repeat(40)
    );
    assert_eq!(
        v.get("env").unwrap().get("LANG").unwrap().str().unwrap(),
        "C.UTF-8"
    );
    assert_eq!(
        v.get("env")
            .unwrap()
            .get("GIT_SHA1")
            .unwrap()
            .str()
            .unwrap(),
        "c".repeat(40)
    );
}

#[test]
fn push_requires_revision_and_valid_explicit_tags() {
    let f = Fixture::new();
    f.write("plan.json", &plan());
    f.script("git", "exit 1");
    assert!(!f
        .run(&["container", "plan.json", "push", "registry.test/demo"])
        .status
        .success());
    f.script("git", &format!("echo {}", "c".repeat(40)));
    let o = f.run(&[
        "container",
        "plan.json",
        "push",
        "registry.test/demo",
        "--tag",
        "bad/tag",
    ]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("invalid image tag"));
}

#[test]
fn base_validate_passes_pin_and_writes_result() {
    let f = Fixture::new();
    f.spec();
    f.write("image.lock.json", &base());
    f.script(
        "nix",
        r#"printf '%s\n' "$*" >> "$LOG"
echo '[{"outputs":{"out":"/nix/store/00000000000000000000000000000000-base"}}]'"#,
    );
    success(&f.run(&[
        "base-image",
        "base.json",
        "validate",
        "--out",
        "validated.txt",
    ]));
    assert!(fs::read_to_string(f.path("validated.txt"))
        .unwrap()
        .ends_with("-base\n"));
    let log = fs::read_to_string(f.path("log")).unwrap();
    assert!(log.contains("aarch64-linux"));
    assert!(log.contains("sha256-E"));
}

#[test]
fn import_propagates_nix_failure_without_replacing_result() {
    let f = Fixture::new();
    fs::write(f.path("artifact"), b"hello").unwrap();
    fs::write(f.path("result"), b"previous\n").unwrap();
    f.write(
        "import.json",
        &versioned([
            ("name", "hello".into()),
            ("artifact", "artifact".into()),
            ("import_backend", "derivation".into()),
        ]),
    );
    f.script(
        "nix",
        r#"case "$3" in nar) printf 'fake-nar';; build) exit 9;; *) exit 97;; esac"#,
    );
    let o = f.run(&["import", "import.json", "result"]);
    assert!(!o.status.success());
    assert_eq!(fs::read(f.path("result")).unwrap(), b"previous\n")
}

#[test]
fn version_is_checked_before_tools_run() {
    let f = Fixture::new();
    let mut p = plan();
    p.set("schema_version", V::Number("9".into())).unwrap();
    f.write("plan.json", &p);
    f.script("git", &format!("echo {}", "c".repeat(40)));
    let o = f.run(&["container", "plan.json", "plan"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("schema_version"));
}

#[test]
fn failed_save_preserves_existing_archive() {
    let f = Fixture::new();
    f.script("git", &format!("echo {}", "c".repeat(40)));
    f.script("nix", "exit 2");
    f.write("plan.json", &plan());
    fs::write(f.path("image.tar"), b"original").unwrap();
    assert!(!f
        .run(&["container", "plan.json", "save", "image.tar"])
        .status
        .success());
    assert_eq!(fs::read(f.path("image.tar")).unwrap(), b"original")
}

#[test]
fn malformed_pin_is_rejected_before_nix() {
    let f = Fixture::new();
    f.spec();
    let mut v = base();
    v.set("image_digest", "sha256:bad".into()).unwrap();
    f.write("image.lock.json", &v);
    let o = f.run(&["base-image", "base.json", "validate"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("invalid sha256"));
}

#[test]
fn prefetch_requests_machine_readable_json() {
    let f = Fixture::new();
    f.spec();
    f.fake_registry();
    success(&f.run(&["base-image", "base.json", "update"]));
    let log = fs::read_to_string(f.path("log")).unwrap();
    let inspect = log.lines().find(|l| l.contains("inspect")).unwrap();
    assert!(inspect.contains("--override-os linux"));
    assert!(inspect.contains("--override-arch arm64"));
    assert!(log
        .lines()
        .filter(|l| l.contains("--image-name"))
        .all(|l| l.contains("--json")));
}

#[test]
fn direct_is_default_without_silent_backend_fallback() {
    let f = Fixture::new();
    fs::write(f.path("artifact"), b"bytes").unwrap();
    fs::write(f.path("result"), b"previous\n").unwrap();
    let mut spec = versioned([("name", "app".into()), ("artifact", "artifact".into())]);
    f.write("spec.json", &spec);
    f.script(
        "nix",
        r#"printf '%s\n' "$*" >> "$LOG"
case "$3" in nar) printf fake-nar;; *) exit 99;; esac"#,
    );
    let result = f.run(&["import", "spec.json", "result"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("connect to Nix daemon"));
    assert!(!fs::read_to_string(f.path("log")).unwrap().contains("build"));
    assert_eq!(fs::read(f.path("result")).unwrap(), b"previous\n");
    spec.set("import_backend", "unsupported".into()).unwrap();
    f.write("spec.json", &spec);
    let result = f.run(&["import", "spec.json", "result"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("import_backend must be"));
}
