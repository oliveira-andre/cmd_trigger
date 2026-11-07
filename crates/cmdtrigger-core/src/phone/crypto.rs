//! The phone link's cryptography, mirrored in the iPhone app (`ios/CmdTrigger/Net/PhoneCrypto.swift`).
//!
//! Pairing (the code is 16 Crockford base-32 characters, 80 bits, shown on the Mac as a QR code
//! and as text; it is never sent):
//!
//! ```text
//! codeKey   = SHA-256("cmdtrigger-pair-v1:" + code)
//! phone  →  phoneId, phoneName, phonePublic (X25519),
//!           proof = HMAC-SHA256(codeKey, "phone" ‖ phonePublic ‖ phoneId)
//! mac    →  macId, macName, macPublic (X25519, fresh),
//!           proof = HMAC-SHA256(codeKey, "mac" ‖ macPublic ‖ phonePublic)
//! key       = HKDF-SHA256(salt = codeKey, ikm = X25519(shared),
//!                         info = "cmdtrigger-phone-v1" ‖ phonePublic ‖ macPublic)   (32 bytes)
//! ```
//!
//! Afterwards every call and reply is ChaCha20-Poly1305 with that key: `nonce(12) ‖ ciphertext ‖
//! tag(16)` (CryptoKit's `ChaChaPoly.SealedBox.combined`), with the associated data
//! `"cmdtrigger-phone-v1:call:<phoneId>"` or `…:reply:<phoneId>`.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// Crockford base 32: no I, L, O or U, so it reads and types unambiguously.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
pub const CODE_LEN: usize = 16;

pub fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the OS has randomness");
    bytes
}

/// A fresh 80-bit pairing code, 16 characters.
pub fn new_code() -> String {
    let bytes = random::<10>();
    let mut bits = 0u32;
    let mut count = 0;
    let mut out = String::with_capacity(CODE_LEN);
    for b in bytes {
        bits = (bits << 8) | b as u32;
        count += 8;
        while count >= 5 {
            count -= 5;
            out.push(ALPHABET[((bits >> count) & 31) as usize] as char);
        }
    }
    out
}

/// "ABCD-EFGH-JKMN-PQRS" for reading off the screen.
pub fn grouped(code: &str) -> String {
    code.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap_or("")).collect::<Vec<_>>().join("-")
}

/// What the user typed → the canonical code: dashes and spaces dropped, upper case, and the
/// look-alikes O→0, I/L→1. `None` unless it's 16 valid characters.
pub fn normalize_code(typed: &str) -> Option<String> {
    let code: String = typed
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            other => other,
        })
        .collect();
    (code.len() == CODE_LEN && code.bytes().all(|b| ALPHABET.contains(&b))).then_some(code)
}

pub fn code_key(code: &str) -> [u8; 32] {
    Sha256::digest(format!("cmdtrigger-pair-v1:{code}").as_bytes()).into()
}

fn hmac(key: &[u8; 32], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

pub fn phone_proof(code_key: &[u8; 32], phone_public: &[u8; 32], phone_id: &str) -> [u8; 32] {
    hmac(code_key, &[b"phone", phone_public, phone_id.as_bytes()])
}

pub fn mac_proof(code_key: &[u8; 32], mac_public: &[u8; 32], phone_public: &[u8; 32]) -> [u8; 32] {
    hmac(code_key, &[b"mac", mac_public, phone_public])
}

pub fn derive_key(code_key: &[u8; 32], shared: &[u8; 32], phone_public: &[u8; 32], mac_public: &[u8; 32]) -> [u8; 32] {
    let mut info = b"cmdtrigger-phone-v1".to_vec();
    info.extend_from_slice(phone_public);
    info.extend_from_slice(mac_public);
    let mut out = [0u8; 32];
    Hkdf::<Sha256>::new(Some(code_key), shared).expand(&info, &mut out).expect("32 bytes is a valid HKDF length");
    out
}

/// The Mac's side of pairing: a fresh key pair, the shared key, and the proof for the phone.
pub fn mac_side(code_key: &[u8; 32], phone_public: &[u8; 32]) -> ([u8; 32], [u8; 32], [u8; 32]) {
    mac_side_with(code_key, phone_public, random::<32>())
}

/// [`mac_side`] with a chosen secret, for test vectors.
pub fn mac_side_with(code_key: &[u8; 32], phone_public: &[u8; 32], secret: [u8; 32]) -> ([u8; 32], [u8; 32], [u8; 32]) {
    let secret = x25519_dalek::StaticSecret::from(secret);
    let mac_public = x25519_dalek::PublicKey::from(&secret).to_bytes();
    let shared = secret.diffie_hellman(&x25519_dalek::PublicKey::from(*phone_public)).to_bytes();
    let key = derive_key(code_key, &shared, phone_public, &mac_public);
    (mac_public, key, mac_proof(code_key, &mac_public, phone_public))
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    crate::homekit::srp::constant_time_eq(a, b)
}

fn aad(direction: &str, phone_id: &str) -> Vec<u8> {
    format!("cmdtrigger-phone-v1:{direction}:{phone_id}").into_bytes()
}

/// `nonce ‖ ciphertext ‖ tag`.
pub fn seal(key: &[u8; 32], direction: &str, phone_id: &str, plaintext: &[u8]) -> Vec<u8> {
    seal_with_nonce(key, direction, phone_id, plaintext, random::<12>())
}

pub fn seal_with_nonce(key: &[u8; 32], direction: &str, phone_id: &str, plaintext: &[u8], nonce: [u8; 12]) -> Vec<u8> {
    let aad = aad(direction, phone_id);
    let sealed = ChaCha20Poly1305::new(&Key::from(*key))
        .encrypt(&Nonce::from(nonce), Payload { msg: plaintext, aad: &aad })
        .expect("ChaCha20-Poly1305 encryption cannot fail");
    [nonce.as_slice(), &sealed].concat()
}

/// The plaintext and the nonce (for replay checks), or `None` if it was forged or for
/// someone else.
pub fn open(key: &[u8; 32], direction: &str, phone_id: &str, combined: &[u8]) -> Option<(Vec<u8>, [u8; 12])> {
    if combined.len() < 12 + 16 {
        return None;
    }
    let nonce: [u8; 12] = combined[..12].try_into().ok()?;
    let aad = aad(direction, phone_id);
    let plain = ChaCha20Poly1305::new(&Key::from(*key))
        .decrypt(&Nonce::from(nonce), Payload { msg: &combined[12..], aad: &aad })
        .ok()?;
    Some((plain, nonce))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed inputs; the same numbers are checked by the iPhone app's PhoneCryptoTests.
    pub(crate) fn vectors() -> serde_json::Value {
        let code = "7K3QH2MZP9WC4T8B";
        let ck = code_key(code);
        let phone_secret = x25519_dalek::StaticSecret::from([0x11u8; 32]);
        let phone_public = x25519_dalek::PublicKey::from(&phone_secret).to_bytes();
        let phone_id = "PHONE-1";
        let (mac_public, key, mac_proof) = mac_side_with(&ck, &phone_public, [0x22u8; 32]);
        let sealed = seal_with_nonce(&key, "call", phone_id, br#"{"op":"status"}"#, [0x33u8; 12]);
        serde_json::json!({
            "code": code,
            "codeKey": hex::encode(ck),
            "phoneSecret": hex::encode([0x11u8; 32]),
            "phonePublic": hex::encode(phone_public),
            "phoneId": phone_id,
            "phoneProof": hex::encode(phone_proof(&ck, &phone_public, phone_id)),
            "macSecret": hex::encode([0x22u8; 32]),
            "macPublic": hex::encode(mac_public),
            "macProof": hex::encode(mac_proof),
            "key": hex::encode(key),
            "plaintext": r#"{"op":"status"}"#,
            "nonce": hex::encode([0x33u8; 12]),
            "sealedCall": hex::encode(sealed),
        })
    }

    #[test]
    fn codes_are_80_bits_of_crockford() {
        let code = new_code();
        assert_eq!(code.len(), CODE_LEN);
        assert_eq!(normalize_code(&grouped(&code)).as_deref(), Some(code.as_str()));
        assert_eq!(normalize_code("7k3q-h2mz-p9wc-4t8b").as_deref(), Some("7K3QH2MZP9WC4T8B"));
        assert_eq!(normalize_code("OIL0-0000-0000-0000").as_deref(), Some("0110000000000000"));
        assert_eq!(normalize_code("7K3Q-H2MZ"), None);
        assert_eq!(normalize_code("UUUU-UUUU-UUUU-UUUU"), None, "U isn't in the alphabet");
        assert_ne!(new_code(), new_code());
    }

    #[test]
    fn both_sides_derive_the_same_key_and_seal_round_trips() {
        let ck = code_key("7K3QH2MZP9WC4T8B");
        let phone_secret = x25519_dalek::StaticSecret::from(random::<32>());
        let phone_public = x25519_dalek::PublicKey::from(&phone_secret).to_bytes();
        let (mac_public, mac_key, proof) = mac_side(&ck, &phone_public);
        let shared = phone_secret.diffie_hellman(&x25519_dalek::PublicKey::from(mac_public)).to_bytes();
        let phone_key = derive_key(&ck, &shared, &phone_public, &mac_public);
        assert_eq!(phone_key, mac_key);
        assert_eq!(proof, mac_proof(&ck, &mac_public, &phone_public));

        let sealed = seal(&mac_key, "reply", "p1", b"hello");
        assert_eq!(open(&phone_key, "reply", "p1", &sealed).unwrap().0, b"hello");
        assert!(open(&phone_key, "call", "p1", &sealed).is_none(), "direction is authenticated");
        assert!(open(&phone_key, "reply", "p2", &sealed).is_none(), "phone id is authenticated");
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(open(&phone_key, "reply", "p1", &tampered).is_none());
        assert!(open(&[0u8; 32], "reply", "p1", &sealed).is_none());
    }

    #[test]
    fn vectors_are_stable() {
        let v = vectors();
        // Regenerate ios/CmdTriggerTests/phone_vectors.json with
        // `PHONE_VECTORS=1 cargo test -p cmdtrigger-core phone::crypto` when the scheme changes.
        if std::env::var_os("PHONE_VECTORS").is_some() {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../ios/CmdTriggerTests/phone_vectors.json");
            std::fs::write(path, serde_json::to_string_pretty(&v).unwrap() + "\n").unwrap();
        }
        let saved = include_str!("../../../../ios/CmdTriggerTests/phone_vectors.json");
        let saved: serde_json::Value = serde_json::from_str(saved).unwrap();
        assert_eq!(saved, v, "the iPhone app's vectors no longer match");
    }
}
