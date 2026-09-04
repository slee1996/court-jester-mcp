//! Stable source and embedded-tree hashes shared by verification and replay.

use crate::types::EmbeddedSource;
use sha2::{Digest, Sha256};

pub(super) fn stable_digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub(super) fn tree_digest(files: &[EmbeddedSource]) -> String {
    if files.len() == 1 {
        return stable_digest(&files[0].content);
    }
    let mut entries = files
        .iter()
        .map(|source| format!("{}\n{}", source.relative_path, source.content))
        .collect::<Vec<_>>();
    entries.sort();
    stable_digest(&entries.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::stable_digest;

    #[test]
    fn source_digest_preserves_standard_sha256_vectors() {
        assert_eq!(
            stable_digest(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            stable_digest("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            stable_digest(&"a".repeat(1_000_000)),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }
}
