use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    fs::File,
    io::{self, Read},
    path::Path,
};

/// A typed SHA-256 result. Hex encoding is explicit at serialization boundaries.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    pub fn of_bytes(bytes: &[u8]) -> Self {
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        Self(digest)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Sha256Digest")
            .field(&self.to_hex())
            .finish()
    }
}

pub fn sha256(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::of_bytes(bytes)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    sha256(bytes).to_hex()
}

/// Hash a sequence of byte chunks exactly as if they had been concatenated.
pub fn sha256_chunks<I, B>(chunks: I) -> Sha256Digest
where
    I: IntoIterator<Item = B>,
    B: AsRef<[u8]>,
{
    let mut hasher = Sha256::new();
    for chunk in chunks {
        hasher.update(chunk.as_ref());
    }
    Sha256Digest(hasher.finalize().into())
}

/// Hash a stream without buffering the entire artifact in memory.
pub fn sha256_reader<R: Read>(mut reader: R) -> io::Result<Sha256Digest> {
    sha256_reader_bounded(&mut reader, u64::MAX)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "SHA-256 input exceeded the supported stream length",
        )
    })
}

/// Hash a stream while enforcing a byte ceiling. `None` means the stream
/// exceeded `max_bytes`; I/O failures remain distinct errors.
pub fn sha256_reader_bounded<R: Read>(
    mut reader: R,
    max_bytes: u64,
) -> io::Result<Option<Sha256Digest>> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let Some(next_total) = total.checked_add(read as u64) else {
            return Ok(None);
        };
        if next_total > max_bytes {
            return Ok(None);
        }
        total = next_total;
        hasher.update(&buffer[..read]);
    }
    Ok(Some(Sha256Digest(hasher.finalize().into())))
}

pub fn sha256_file_hex(path: &Path) -> io::Result<String> {
    sha256_reader(File::open(path)?).map(Sha256Digest::to_hex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Cursor};

    #[test]
    fn matches_nist_sha256_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn typed_digest_exposes_exact_32_bytes() {
        assert_eq!(sha256(b"oomu").as_bytes().len(), 32);
    }

    #[test]
    fn streaming_and_file_hashes_match_known_vector() {
        let expected = sha256_hex(b"abc");
        assert_eq!(
            sha256_reader(Cursor::new(b"abc")).unwrap().to_hex(),
            expected
        );

        let path = std::env::temp_dir().join(format!(
            "oomu-foundation-digest-{}-{}.txt",
            std::process::id(),
            crate::foundation::clock::unix_time_ms_u128()
        ));
        fs::write(&path, b"abc").unwrap();
        assert_eq!(sha256_file_hex(&path).unwrap(), expected);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn chunked_and_bounded_hashing_preserve_exact_bytes() {
        assert_eq!(
            sha256_chunks([b"a".as_slice(), b"bc".as_slice()]),
            sha256(b"abc")
        );
        assert_eq!(
            sha256_reader_bounded(Cursor::new(b"abc"), 3)
                .unwrap()
                .unwrap(),
            sha256(b"abc")
        );
        assert_eq!(sha256_reader_bounded(Cursor::new(b"abc"), 2).unwrap(), None);
    }
}
