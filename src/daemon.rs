//! Narrow Nix worker-protocol client for content-addressed NAR imports.
//!
//! Protocol 1.25 deliberately avoids newer optional encodings while retaining
//! AddToStore's explicit references. See Nix 2.34.7 worker-protocol definitions.
//! No Nix libraries, Lix executable, hashing implementation, or builder needed.

use crate::{workflow::store_path, Result};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{self, BufReader, BufWriter, Read, Write},
    net::Shutdown,
    os::unix::{fs::symlink, net::UnixStream},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
};

const VERSION: u64 = 0x119; // major 1, minor 25

const MAGIC_CLIENT: u64 = 0x6e697863;

const MAGIC_SERVER: u64 = 0x6478696f;

const LAST: u64 = 0x616c7473;

const ERROR: u64 = 0x63787470;

const NEXT: u64 = 0x6f6c6d67;

const START: u64 = 0x53545254;

const STOP: u64 = 0x53544f50;

const RESULT: u64 = 0x52534c54;

const MAX_FIELD: u64 = 16 * 1024 * 1024;

static ROOT_ID: AtomicU64 = AtomicU64::new(0);

fn write_u64(out: &mut impl Write, n: u64) -> io::Result<()> {
    out.write_all(&n.to_le_bytes())
}

fn read_u64(input: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0; 8];
    input.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn write_string(out: &mut impl Write, value: &str) -> io::Result<()> {
    write_u64(out, value.len() as u64)?;
    out.write_all(value.as_bytes())?;
    out.write_all(&[0; 7][..(8 - value.len() % 8) % 8])
}

fn read_string(input: &mut impl Read) -> Result<String> {
    let len = read_u64(input)?;
    if len > MAX_FIELD {
        return Err("oversized Nix protocol string".into());
    }
    let mut bytes = vec![0; len as usize];
    input.read_exact(&mut bytes)?;
    let mut padding = [0; 7];
    let n = ((8 - len % 8) % 8) as usize;
    input.read_exact(&mut padding[..n])?;
    if padding[..n].iter().any(|b| *b != 0) {
        return Err("invalid Nix string padding".into());
    }
    Ok(String::from_utf8(bytes)?)
}

fn read_count(input: &mut impl Read) -> Result<u64> {
    let n = read_u64(input)?;
    if n > 1_000_000 {
        return Err("oversized Nix protocol collection".into());
    }
    Ok(n)
}

fn strings(input: &mut impl Read) -> Result<BTreeSet<String>> {
    let mut values = BTreeSet::new();
    for _ in 0..read_count(input)? {
        values.insert(read_string(input)?);
    }
    Ok(values)
}

fn fields(input: &mut impl Read) -> Result<()> {
    for _ in 0..read_count(input)? {
        match read_u64(input)? {
            0 => {
                read_u64(input)?;
            }
            1 => {
                read_string(input)?;
            }
            _ => return Err("unknown Nix activity field type".into()),
        }
    }
    Ok(())
}

fn messages(input: &mut impl Read) -> Result<()> {
    loop {
        match read_u64(input)? {
            LAST => return Ok(()),
            ERROR => {
                let message = read_string(input)?;
                let status = read_u64(input)?;
                return Err(format!("Nix daemon ({status}): {message}").into());
            }
            NEXT => eprint!("{}", read_string(input)?),
            START => {
                read_u64(input)?;
                read_u64(input)?;
                read_u64(input)?;
                read_string(input)?;
                fields(input)?;
                read_u64(input)?;
            }
            STOP => {
                read_u64(input)?;
            }
            RESULT => {
                read_u64(input)?;
                read_u64(input)?;
                fields(input)?;
            }
            n => return Err(format!("unsupported Nix daemon message {n:#x}").into()),
        }
    }
}

struct Connection {
    input: BufReader<UnixStream>,
    output: BufWriter<UnixStream>,
}

impl Connection {
    fn connect(socket: UnixStream) -> Result<Self> {
        let mut output = BufWriter::new(socket.try_clone()?);
        let mut input = BufReader::new(socket);
        write_u64(&mut output, MAGIC_CLIENT)?;
        write_u64(&mut output, VERSION)?;
        output.flush()?;
        if read_u64(&mut input)? != MAGIC_SERVER {
            return Err("not a Nix daemon: handshake magic mismatch".into());
        }
        let remote = read_u64(&mut input)?;
        if remote >> 8 != 1 || remote < VERSION {
            return Err(format!("direct imports require Nix daemon protocol >= 1.25, got {remote:#x}; select the derivation backend if needed").into());
        }
        // Legacy affinity and reserve-space fields. We negotiate 1.25, so there
        // are no optional feature sets or post-handshake version strings.
        write_u64(&mut output, 0)?;
        write_u64(&mut output, 0)?;
        output.flush()?;
        messages(&mut input)?;
        Ok(Self { input, output })
    }

    fn add(&mut self, mut nar: File, name: &str, references: &BTreeSet<String>) -> Result<String> {
        let nar_size = nar.metadata()?.len();
        write_u64(&mut self.output, 7)?; // AddToStore
        write_string(&mut self.output, name)?;
        write_string(&mut self.output, "fixed:r:sha256")?;
        write_u64(&mut self.output, references.len() as u64)?;
        for reference in references {
            write_string(&mut self.output, reference)?;
        }
        write_u64(&mut self.output, 0)?; // repair=false; never request privilege bypasses
        self.output.flush()?;
        let stream = self.output.get_ref().try_clone()?;
        // Read progress/errors concurrently: a daemon can fill its socket while
        // receiving a large NAR. On error, shutdown also unblocks the sender.
        let cancel = stream.try_clone()?;
        let sender = thread::spawn(move || -> io::Result<()> {
            let result = (|| {
                let mut out = BufWriter::new(stream);
                let mut buffer = [0; 65536];
                loop {
                    let n = match nar.read(&mut buffer) {
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        other => other?,
                    };
                    write_u64(&mut out, n as u64)?;
                    if n == 0 {
                        break;
                    }
                    out.write_all(&buffer[..n])?;
                }
                out.flush()
            })();
            if result.is_err() {
                let _ = cancel.shutdown(Shutdown::Both);
            }
            result
        });
        let response = messages(&mut self.input);
        if response.is_err() {
            let _ = self.input.get_ref().shutdown(Shutdown::Both);
        }
        let sent = sender.join().map_err(|_| "Nix upload thread panicked")?;
        response?;
        sent?;
        let path = read_string(&mut self.input)?;
        store_path(&path)?;
        let _deriver = read_string(&mut self.input)?;
        let _nar_hash = read_string(&mut self.input)?;
        let returned_refs = strings(&mut self.input)?;
        let _registered = read_u64(&mut self.input)?;
        let size = read_u64(&mut self.input)?;
        let _ultimate = read_u64(&mut self.input)?;
        let _signatures = strings(&mut self.input)?;
        let ca = read_string(&mut self.input)?;
        if returned_refs != *references || size != nar_size || !ca.starts_with("fixed:r:sha256:") {
            return Err(
                "Nix daemon returned inconsistent content-addressed import metadata".into(),
            );
        }
        Ok(path)
    }

    fn root(&mut self, path: &str, root: &Path) -> Result<()> {
        write_u64(&mut self.output, 11)?;
        write_string(&mut self.output, path)?;
        self.output.flush()?;
        messages(&mut self.input)?;
        read_u64(&mut self.input)?;
        let root = std::path::absolute(root)?;
        if let Ok(meta) = fs::symlink_metadata(&root) {
            if !meta.file_type().is_symlink() {
                return Err("refusing to replace a non-symlink GC root".into());
            }
        }
        let parent = root.parent().ok_or("GC root has no parent")?;
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(
            ".coldeer-root-{}-{}",
            std::process::id(),
            ROOT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        symlink(path, &temporary)?;
        if let Err(e) = fs::rename(&temporary, &root) {
            let _ = fs::remove_file(temporary);
            return Err(e.into());
        }
        write_u64(&mut self.output, 12)?;
        write_string(
            &mut self.output,
            root.to_str().ok_or("non-UTF8 GC-root path")?,
        )?;
        self.output.flush()?;
        messages(&mut self.input)?;
        read_u64(&mut self.input)?;
        Ok(())
    }
}

fn socket_path() -> Result<PathBuf> {
    let remote = std::env::var("NIX_REMOTE").unwrap_or_default();
    if let Some(path) = remote.strip_prefix("unix://") {
        if path.contains('?') {
            return Err("direct imports do not support Unix-store URI options; use import_backend=derivation".into());
        }
        return Ok(path.into());
    }
    if !matches!(remote.as_str(), "" | "auto" | "daemon") {
        return Err("direct imports require a local Nix daemon; select import_backend=derivation for other store transports".into());
    }
    Ok(std::env::var_os("NIX_DAEMON_SOCKET_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/nix/var/nix/daemon-socket/socket")))
}

pub fn import(
    nar: &Path,
    name: &str,
    references: &BTreeSet<String>,
    root: &Path,
) -> Result<String> {
    let socket = socket_path()?;
    let socket = UnixStream::connect(&socket)
        .map_err(|e| format!("connect to Nix daemon {}: {e}", socket.display()))?;
    let mut connection = Connection::connect(socket)?;
    let path = connection.add(File::open(nar)?, name, references)?;
    connection.root(&path, root)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const PATH: &str = "/nix/store/00000000000000000000000000000000-import";

    const DEP: &str = "/nix/store/11111111111111111111111111111111-dependency";

    #[test]
    #[ignore = "starts an isolated real Nix daemon; requires nix on PATH"]
    fn real_daemon_without_ca_derivations() {
        use std::process::{Command, Stdio};

        // Keep this bidirectional proxy interactive. Linux's optimized
        // io::copy splice path can wait for more pipe data before forwarding
        // a short protocol message to the socket, deadlocking the handshake.
        fn forward(mut input: impl Read, mut output: impl Write) -> io::Result<()> {
            let mut buffer = [0; 8192];
            loop {
                let n = match input.read(&mut buffer) {
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    other => other?,
                };
                if n == 0 {
                    return Ok(());
                }
                output.write_all(&buffer[..n])?;
            }
        }

        let directory = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "coldeer-real-daemon-{}-{}",
                std::process::id(),
                ROOT_ID.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir(&directory).unwrap();
        let store_root = directory.join("store");
        let mut child = Command::new("nix")
            .arg("--store")
            .arg(format!("local?root={}", store_root.display()))
            .args([
                "--option",
                "experimental-features",
                "nix-command daemon-trust-override",
                "daemon",
                "--stdio",
                "--force-untrusted",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let (client, proxy) = UnixStream::pair().unwrap();
        let mut to_daemon = proxy.try_clone().unwrap();
        let mut input = child.stdin.take().unwrap();
        let input_thread = thread::spawn(move || forward(&mut to_daemon, &mut input));
        let mut from_daemon = child.stdout.take().unwrap();
        let output_thread = thread::spawn(move || {
            let result = forward(&mut from_daemon, &mut &proxy);
            let _ = proxy.shutdown(Shutdown::Write);
            result
        });
        let mut conn = Connection::connect(client).unwrap();
        let artifact = directory.join("artifact");
        let nar = directory.join("artifact.nar");
        fs::write(&artifact, b"dependency bytes").unwrap();
        let dump = || {
            assert!(Command::new("nix")
                .args([
                    "--extra-experimental-features",
                    "nix-command",
                    "nar",
                    "dump-path"
                ])
                .arg(&artifact)
                .stdout(File::create(&nar).unwrap())
                .status()
                .unwrap()
                .success());
        };
        dump();
        let dependency = conn
            .add(File::open(&nar).unwrap(), "dependency", &BTreeSet::new())
            .unwrap();
        fs::write(&artifact, format!("uses {dependency}\n")).unwrap();
        dump();
        let path = conn
            .add(
                File::open(&nar).unwrap(),
                "app",
                &BTreeSet::from([dependency.clone()]),
            )
            .unwrap();
        assert_eq!(
            fs::read_to_string(store_root.join(path.trim_start_matches('/'))).unwrap(),
            format!("uses {dependency}\n")
        );
        drop(conn);
        input_thread.join().unwrap().unwrap();
        output_thread.join().unwrap().unwrap();
        assert!(child.wait().unwrap().success());
        fs::remove_dir_all(directory).unwrap();
    }

    fn pair() -> (UnixStream, UnixStream) {
        let (a, b) = UnixStream::pair().unwrap();
        for socket in [&a, &b] {
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
        }
        (a, b)
    }

    fn welcome(socket: &mut UnixStream) {
        assert_eq!(read_u64(socket).unwrap(), MAGIC_CLIENT);
        assert_eq!(read_u64(socket).unwrap(), VERSION);
        write_u64(socket, MAGIC_SERVER).unwrap();
        write_u64(socket, 0x126).unwrap();
        assert_eq!(read_u64(socket).unwrap(), 0);
        assert_eq!(read_u64(socket).unwrap(), 0);
        write_u64(socket, LAST).unwrap();
    }

    fn header(socket: &mut UnixStream) {
        assert_eq!(read_u64(socket).unwrap(), 7);
        assert_eq!(read_string(socket).unwrap(), "import");
        assert_eq!(read_string(socket).unwrap(), "fixed:r:sha256");
        assert_eq!(strings(socket).unwrap(), BTreeSet::from([DEP.to_owned()]));
        assert_eq!(read_u64(socket).unwrap(), 0);
    }

    fn source() -> (PathBuf, Vec<u8>) {
        let path = std::env::temp_dir().join(format!(
            "coldeer-wire-{}-{}",
            std::process::id(),
            ROOT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let bytes = (0..1_000_013).map(|i| (i % 251) as u8).collect::<Vec<_>>();
        fs::write(&path, &bytes).unwrap();
        (path, bytes)
    }

    #[test]
    fn framed_import_handles_concurrent_progress_and_metadata() {
        let (client, mut server) = pair();
        let (file, expected) = source();
        let length = expected.len();
        let worker = thread::spawn(move || {
            welcome(&mut server);
            header(&mut server);
            // More than the socket buffer, without diagnostic output noise.
            for _ in 0..8 {
                for n in [RESULT, 1, 101, 1, 1] {
                    write_u64(&mut server, n).unwrap();
                }
                write_string(&mut server, &"x".repeat(65536)).unwrap();
            }
            let mut received = Vec::new();
            loop {
                let n = read_u64(&mut server).unwrap() as usize;
                if n == 0 {
                    break;
                }
                assert!(n <= 65536);
                let start = received.len();
                received.resize(start + n, 0);
                server.read_exact(&mut received[start..]).unwrap();
            }
            assert_eq!(received, expected);
            write_u64(&mut server, LAST).unwrap();
            for s in [PATH, "", &"a".repeat(64)] {
                write_string(&mut server, s).unwrap();
            }
            write_u64(&mut server, 1).unwrap();
            write_string(&mut server, DEP).unwrap();
            for n in [0, length as u64, 0, 0] {
                write_u64(&mut server, n).unwrap();
            }
            write_string(&mut server, "fixed:r:sha256:hash").unwrap();
        });
        let mut conn = Connection::connect(client).unwrap();
        let result = conn.add(
            File::open(&file).unwrap(),
            "import",
            &BTreeSet::from([DEP.into()]),
        );
        fs::remove_file(file).unwrap();
        assert_eq!(result.unwrap(), PATH);
        worker.join().unwrap();
    }

    #[test]
    fn daemon_error_cancels_blocked_sender() {
        let (client, mut server) = pair();
        let (file, _) = source();
        let worker = thread::spawn(move || {
            welcome(&mut server);
            header(&mut server);
            write_u64(&mut server, ERROR).unwrap();
            write_string(&mut server, "missing dependency").unwrap();
            write_u64(&mut server, 1).unwrap();
        });
        let mut conn = Connection::connect(client).unwrap();
        let error = conn
            .add(
                File::open(&file).unwrap(),
                "import",
                &BTreeSet::from([DEP.into()]),
            )
            .unwrap_err();
        fs::remove_file(file).unwrap();
        worker.join().unwrap();
        assert!(error.to_string().contains("missing dependency"));
    }

    #[test]
    fn rejects_old_daemons_and_malformed_wire_values() {
        let (client, mut server) = pair();
        let worker = thread::spawn(move || {
            read_u64(&mut server).unwrap();
            read_u64(&mut server).unwrap();
            write_u64(&mut server, MAGIC_SERVER).unwrap();
            write_u64(&mut server, 0x118).unwrap();
        });
        assert!(Connection::connect(client).is_err());
        worker.join().unwrap();
        let mut bytes = Vec::new();
        write_string(&mut bytes, "abc").unwrap();
        assert_eq!(
            bytes,
            [vec![3, 0, 0, 0, 0, 0, 0, 0], b"abc".to_vec(), vec![0; 5]].concat()
        );
        bytes[15] = 1;
        assert!(read_string(&mut &bytes[..]).is_err());
        assert!(read_string(&mut &(MAX_FIELD + 1).to_le_bytes()[..]).is_err());
        assert!(messages(&mut &9u64.to_le_bytes()[..]).is_err());
    }
}
