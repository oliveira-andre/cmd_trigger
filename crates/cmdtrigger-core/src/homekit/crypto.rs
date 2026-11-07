//! HKDF-SHA512, ChaCha20-Poly1305 and the encrypted session framing HomeKit uses after
//! pair-verify.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use sha2::Sha512;

pub fn hkdf_sha512(ikm: &[u8], salt: &str, info: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    Hkdf::<Sha512>::new(Some(salt.as_bytes()), ikm)
        .expand(info.as_bytes(), &mut out)
        .expect("32 bytes is a valid HKDF-SHA512 length");
    out
}

/// A pairing-message nonce: four zero bytes and an 8-byte label such as `PS-Msg05`.
pub fn label_nonce(label: &[u8; 8]) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[4..].copy_from_slice(label);
    nonce
}

pub fn seal(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    ChaCha20Poly1305::new(&Key::from(*key))
        .encrypt(&Nonce::from(*nonce), Payload { msg: plaintext, aad })
        .expect("ChaCha20-Poly1305 encryption cannot fail")
}

pub fn open(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], ciphertext: &[u8]) -> Option<Vec<u8>> {
    ChaCha20Poly1305::new(&Key::from(*key)).decrypt(&Nonce::from(*nonce), Payload { msg: ciphertext, aad }).ok()
}

pub const MAX_FRAME: usize = 1024;
const TAG: usize = 16;

/// One direction of an encrypted HAP session: 2-byte little-endian length (also the AAD),
/// ciphertext, 16-byte tag; the nonce is a per-direction counter.
/// The encrypted stream was corrupt or forged.
#[derive(Debug, PartialEq, Eq)]
pub struct BadFrame;

pub struct FrameCipher {
    key: [u8; 32],
    counter: u64,
}

impl FrameCipher {
    pub fn new(key: [u8; 32]) -> Self {
        Self { key, counter: 0 }
    }

    fn nonce(&mut self) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[4..].copy_from_slice(&self.counter.to_le_bytes());
        self.counter += 1;
        nonce
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(plaintext.len() + plaintext.len().div_ceil(MAX_FRAME) * (2 + TAG));
        for chunk in plaintext.chunks(MAX_FRAME) {
            let len = (chunk.len() as u16).to_le_bytes();
            let nonce = self.nonce();
            out.extend_from_slice(&len);
            out.extend(seal(&self.key, &nonce, &len, chunk));
        }
        out
    }

    /// Decrypt every complete frame at the front of `buffer`, removing them. `Err` means the
    /// stream is corrupt or forged and the connection must close.
    pub fn decrypt(&mut self, buffer: &mut Vec<u8>) -> Result<Vec<u8>, BadFrame> {
        let mut plain = Vec::new();
        loop {
            if buffer.len() < 2 {
                break;
            }
            let len = u16::from_le_bytes([buffer[0], buffer[1]]) as usize;
            if len > MAX_FRAME {
                return Err(BadFrame);
            }
            if buffer.len() < 2 + len + TAG {
                break;
            }
            let nonce = self.nonce();
            let frame: Vec<u8> = buffer.drain(..2 + len + TAG).collect();
            plain.extend(open(&self.key, &nonce, &frame[..2], &frame[2..]).ok_or(BadFrame)?);
        }
        Ok(plain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_across_boundaries() {
        let key = hkdf_sha512(b"shared", "Control-Salt", "Control-Read-Encryption-Key");
        let mut sender = FrameCipher::new(key);
        let mut receiver = FrameCipher::new(key);
        let message = vec![42u8; 2500];
        let wire = sender.encrypt(&message);
        assert_eq!(wire.len(), 2500 + 3 * 18);
        let mut buffer = wire[..700].to_vec();
        let mut got = receiver.decrypt(&mut buffer).unwrap();
        buffer.extend_from_slice(&wire[700..]);
        got.extend(receiver.decrypt(&mut buffer).unwrap());
        assert_eq!(got, message);
        assert!(buffer.is_empty());
    }

    #[test]
    fn tampering_is_detected() {
        let key = [1u8; 32];
        let mut wire = FrameCipher::new(key).encrypt(b"hello");
        wire[3] ^= 1;
        assert!(FrameCipher::new(key).decrypt(&mut wire).is_err());
    }
}
