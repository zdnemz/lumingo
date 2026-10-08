use std::fs::File;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};
use speech::CancelFlag;

use crate::error::ModelError;

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(64), |mut out, b| {
        // Writing to a String cannot fail.
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// SHA-256 of a file as lowercase hexadecimal, read in blocks so a model of
/// several hundred megabytes never sits in memory. Checks `cancel` per block.
pub(crate) fn sha256_file(path: &Path, cancel: &CancelFlag) -> Result<String, ModelError> {
    let mut file =
        File::open(path).map_err(|e| ModelError::io(format!("opening {}", path.display()), e))?;
    let mut hasher = Sha256::new();
    let mut block = vec![0_u8; 256 * 1024];
    loop {
        if cancel.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        let n = file
            .read(&mut block)
            .map_err(|e| ModelError::io(format!("reading {}", path.display()), e))?;
        if n == 0 {
            break;
        }
        hasher.update(&block[..n]);
    }
    Ok(to_hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_published_sha256_of_abc() {
        // FIPS 180-2 test vector for "abc".
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("abc.txt");
        std::fs::write(&path, b"abc").expect("write");
        assert_eq!(
            sha256_file(&path, &CancelFlag::new()).expect("hash"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn the_empty_file_has_the_known_digest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("empty");
        std::fs::write(&path, b"").expect("write");
        assert_eq!(
            sha256_file(&path, &CancelFlag::new()).expect("hash"),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn a_cancelled_hash_stops() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("x");
        std::fs::write(&path, b"abc").expect("write");
        let cancel = CancelFlag::new();
        cancel.cancel();
        assert!(matches!(
            sha256_file(&path, &cancel),
            Err(ModelError::Cancelled)
        ));
    }
}
