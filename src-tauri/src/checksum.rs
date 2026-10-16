use std::io::Read;
use std::path::Path;

use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

pub fn file_digest(path: &Path, algorithm: &str) -> Result<String, String> {
    match algorithm {
        "md5" => digest::<Md5>(path),
        "sha1" => digest::<Sha1>(path),
        "sha256" => digest::<Sha256>(path),
        "sha512" => digest::<Sha512>(path),
        other => Err(format!("desteklenmeyen algoritma: {other}")),
    }
}

pub fn algorithm_for(hash: &str) -> Option<&'static str> {
    match hash.trim().len() {
        32 => Some("md5"),
        40 => Some("sha1"),
        64 => Some("sha256"),
        128 => Some("sha512"),
        _ => None,
    }
}

fn digest<D: Digest>(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = D::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digests() {
        let p = std::env::temp_dir().join(format!("pulse-sum-{}.txt", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(file_digest(&p, "md5").unwrap(), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(file_digest(&p, "sha1").unwrap(), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            file_digest(&p, "sha256").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        std::fs::remove_file(p).unwrap();
        assert_eq!(algorithm_for(&"a".repeat(64)), Some("sha256"));
    }
}
