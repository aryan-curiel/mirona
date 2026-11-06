//! Optional encryption at rest (NGR-165): XChaCha20-Poly1305 with one 256-bit key per user,
//! kept in the OS keychain by the engine (the store never sees the keychain).
//!
//! What is sealed: recordings, transcripts (line by line, so a live recording still only
//! appends), embeddings, the voice registry and the corrections (`labels.jsonl`). The search
//! index is kept in memory instead of `index.sqlite`, so no plaintext copy of a transcript is
//! left on disk; it is derived data and rebuilt on open anyway, so this replaces `SQLCipher`.
//! `meeting.md` (title, notes, summary) stays readable on purpose: it's the Obsidian-friendly
//! file the user edits.
//! ponytail: while recording, `audio.wav` is plaintext until the call ends and it is
//! compressed and sealed; sealing the live stream needs a streaming cipher in capture.
//!
//! Formats: a sealed file is `MAGIC ‖ nonce ‖ ciphertext`; a sealed line is `enc1:` + hex of
//! `nonce ‖ ciphertext`. Readers take plaintext too, so turning encryption on or off is a
//! rewrite that can be interrupted safely.

use crate::Result;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use std::path::Path;

const MAGIC: &[u8; 8] = b"MIRONA\x01\x00";
const LINE: &str = "enc1:";
const NONCE: usize = 24;

#[derive(Clone, PartialEq, Eq)]
pub struct Key([u8; 32]);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(…)")
    }
}

impl Key {
    pub fn generate() -> Result<Self> {
        let mut k = [0u8; 32];
        getrandom::fill(&mut k).map_err(|e| format!("no randomness for a key: {e}"))?;
        Ok(Self(k))
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self> {
        let v = hex::decode(s.trim()).map_err(|_| "the store key is not hex")?;
        Ok(Self(
            v.try_into().map_err(|_| "the store key is not 32 bytes")?,
        ))
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new((&self.0).into())
    }

    /// `nonce ‖ ciphertext`.
    fn seal_raw(&self, plain: &[u8]) -> Result<Vec<u8>> {
        let mut nonce = [0u8; NONCE];
        getrandom::fill(&mut nonce).map_err(|e| e.to_string())?;
        let ct = self
            .cipher()
            .encrypt(XNonce::from_slice(&nonce), plain)
            .map_err(|_| "encryption failed")?;
        Ok([nonce.as_slice(), &ct].concat())
    }

    fn open_raw(&self, sealed: &[u8]) -> Result<Vec<u8>> {
        if sealed.len() < NONCE {
            return Err("sealed data is too short".into());
        }
        let (nonce, ct) = sealed.split_at(NONCE);
        self.cipher()
            .decrypt(XNonce::from_slice(nonce), ct)
            .map_err(|_| "can't decrypt: wrong key or damaged file".into())
    }
}

pub fn is_sealed(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// File contents as stored: sealed with `key`, or as they are without one.
pub fn seal(key: Option<&Key>, plain: &[u8]) -> Result<Vec<u8>> {
    match key {
        Some(k) => Ok([MAGIC.as_slice(), &k.seal_raw(plain)?].concat()),
        None => Ok(plain.to_vec()),
    }
}

/// File contents as read: plaintext passes through; sealed needs the key.
pub fn open(key: Option<&Key>, bytes: Vec<u8>) -> Result<Vec<u8>> {
    let Some(sealed) = bytes.strip_prefix(MAGIC) else {
        return Ok(bytes);
    };
    key.ok_or("this file is encrypted and no key is available (keychain)")?
        .open_raw(sealed)
}

/// One line of a JSONL file as stored.
pub fn seal_line(key: Option<&Key>, line: &str) -> Result<String> {
    match key {
        Some(k) => Ok(format!(
            "{LINE}{}",
            hex::encode(k.seal_raw(line.as_bytes())?)
        )),
        None => Ok(line.to_string()),
    }
}

/// One stored line as text.
pub fn open_line(key: Option<&Key>, line: &str) -> Result<String> {
    let Some(h) = line.strip_prefix(LINE) else {
        return Ok(line.to_string());
    };
    let raw = hex::decode(h.trim()).map_err(|_| "damaged encrypted line")?;
    let plain = key
        .ok_or("this file is encrypted and no key is available (keychain)")?
        .open_raw(&raw)?;
    String::from_utf8(plain).map_err(|_| "damaged encrypted line".into())
}

/// Read a file as text, decrypting it if needed. `None` when it doesn't exist.
pub fn read_string(key: Option<&Key>, path: &Path) -> Result<Option<String>> {
    match std::fs::read(path) {
        Ok(b) => {
            let plain = open(key, b).map_err(|e| format!("{}: {e}", path.display()))?;
            String::from_utf8(plain)
                .map(Some)
                .map_err(|_| format!("{}: not text", path.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Rewrite a whole file sealed (`key`) or plain (`None`), atomically. Missing files are fine.
pub fn reseal_file(from: Option<&Key>, to: Option<&Key>, path: &Path) -> Result<()> {
    let Ok(bytes) = std::fs::read(path) else {
        return Ok(());
    };
    let plain = open(from.or(to), bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let tmp = path.with_extension("sealing");
    std::fs::write(&tmp, seal(to, &plain)?).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Rewrite a JSONL file line by line, sealed (`to`) or plain.
pub fn reseal_lines(from: Option<&Key>, to: Option<&Key>, path: &Path) -> Result<()> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    let mut out = String::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let plain = open_line(from.or(to), line).map_err(|e| format!("{}: {e}", path.display()))?;
        out += &seal_line(to, &plain)?;
        out += "\n";
    }
    crate::files::write_atomic(path, &out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_and_lines_round_trip_and_need_the_key() {
        let k = Key::generate().unwrap();
        let other = Key::generate().unwrap();
        assert_eq!(Key::from_hex(&k.to_hex()).unwrap(), k);

        let sealed = seal(Some(&k), b"the password is hunter2").unwrap();
        assert!(is_sealed(&sealed));
        assert!(!sealed.windows(7).any(|w| w == b"hunter2"));
        assert_eq!(
            open(Some(&k), sealed.clone()).unwrap(),
            b"the password is hunter2"
        );
        assert!(open(Some(&other), sealed.clone()).is_err());
        assert!(open(None, sealed).is_err());
        assert_eq!(open(None, b"plain".to_vec()).unwrap(), b"plain");

        let line = seal_line(Some(&k), r#"{"text":"salary 182,000"}"#).unwrap();
        assert!(line.starts_with("enc1:") && !line.contains("salary"));
        assert_eq!(
            open_line(Some(&k), &line).unwrap(),
            r#"{"text":"salary 182,000"}"#
        );
        assert_eq!(open_line(None, "{}").unwrap(), "{}");
        assert!(open_line(None, &line).is_err());

        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.jsonl");
        std::fs::write(&f, "one\ntwo\n").unwrap();
        reseal_lines(None, Some(&k), &f).unwrap();
        let t = std::fs::read_to_string(&f).unwrap();
        assert!(t.lines().all(|l| l.starts_with("enc1:")), "{t}");
        reseal_lines(Some(&k), None, &f).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "one\ntwo\n");
        let g = dir.path().join("audio.caf");
        std::fs::write(&g, b"OggS...").unwrap();
        reseal_file(None, Some(&k), &g).unwrap();
        assert!(is_sealed(&std::fs::read(&g).unwrap()));
        reseal_file(None, Some(&k), &g).unwrap(); // already sealed (an interrupted run): fine
        reseal_file(Some(&k), None, &g).unwrap();
        assert_eq!(std::fs::read(&g).unwrap(), b"OggS...");
    }
}
