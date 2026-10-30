//! Streaming Nix reference detection, independent of the build frontend.
use std::collections::{BTreeSet, HashMap};
use std::io::{self, Read};

/// Find candidate store hashes in arbitrary binary NAR data.
/// Retains overlap so matches spanning read boundaries are included.
pub fn scan_references(
    mut input: impl Read,
    candidates: &[String],
) -> io::Result<BTreeSet<String>> {
    let mut patterns: HashMap<[u8; 32], Vec<&String>> = HashMap::new();
    for path in candidates {
        let name = path.strip_prefix("/nix/store/").ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "expected /nix/store path")
        })?;
        let hash = name.split_once('-').map(|x| x.0).unwrap_or("");
        if hash.len() != 32
            || !hash
                .bytes()
                .all(|b| b"0123456789abcdfghijklmnpqrsvwxyz".contains(&b))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid Nix store hash",
            ));
        }
        patterns
            .entry(hash.as_bytes().try_into().unwrap())
            .or_default()
            .push(path);
    }
    let mut found = BTreeSet::new();
    let mut pending = Vec::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = match input.read(&mut buffer) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            other => other?,
        };
        if n == 0 {
            break;
        }
        pending.extend_from_slice(&buffer[..n]);
        let mut run = 0;
        for (i, byte) in pending.iter().enumerate() {
            run = if b"0123456789abcdfghijklmnpqrsvwxyz".contains(byte) {
                run + 1
            } else {
                0
            };
            if run >= 32 {
                let hash: &[u8; 32] = pending[i + 1 - 32..i + 1].try_into().unwrap();
                if let Some(paths) = patterns.get(hash) {
                    found.extend(paths.iter().map(|p| (*p).clone()));
                }
            }
        }
        let keep = pending.len().saturating_sub(31);
        pending.drain(..keep);
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "00000000000000000000000000000000";

    #[test]
    fn binary_boundary_and_duplicates() {
        let path = format!("/nix/store/{HASH}-demo");
        let mut data = vec![255; 65520];
        data.extend_from_slice(HASH.as_bytes());
        data.extend_from_slice(HASH.as_bytes());
        assert_eq!(
            scan_references(&data[..], std::slice::from_ref(&path)).unwrap(),
            BTreeSet::from([path])
        );
    }

    #[test]
    fn empty_and_invalid() {
        assert!(scan_references(&b""[..], &[]).unwrap().is_empty());
        assert!(scan_references(&b""[..], &["bad".into()]).is_err());
    }
}

pub type Error = Box<dyn std::error::Error + Send + Sync>;

pub type Result<T> = std::result::Result<T, Error>;

pub mod daemon;
pub mod json;
pub mod layering;
pub mod workflow;
