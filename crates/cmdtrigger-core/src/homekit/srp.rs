//! SRP-6a as HomeKit pair-setup uses it: the RFC 5054 3072-bit group, g = 5, SHA-512,
//! username "Pair-Setup" and the setup code as the password.
//!
//! Padding follows hap-nodejs's fast-srp-hap exactly (and the vectors in the tests come from
//! it): `k = H(N | PAD(g))`, `u = H(PAD(A) | PAD(B))`, `K = H(PAD(S))`,
//! `M1 = H(H(N) xor H(g) | H(I) | s | A | B | K)` with unpadded `N` and `g` in the first term,
//! `M2 = H(A | M1 | K)`.

use num_bigint::BigUint;
use num_traits::Zero;
use sha2::{Digest, Sha512};

const N_HEX: [&str; 8] = [
    "ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404dd",
    "ef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7ed",
    "ee386bfb5a899fa5ae9f24117c4b1fe649286651ece45b3dc2007cb8a163bf0598da48361c55d39a69163fa8fd24cf5f",
    "83655d23dca3ad961c62f356208552bb9ed529077096966d670c354e4abc9804f1746c08ca18217c32905e462e36ce3b",
    "e39e772c180e86039b2783a2ec07a28fb5c55df06f4c52c9de2bcbf6955817183995497cea956ae515d2261898fa0510",
    "15728e5a8aaac42dad33170d04507a33a85521abdf1cba64ecfb850458dbef0a8aea71575d060c7db3970f85a6e1e4c7",
    "abf5ae8cdb0933d71e8c94e04a25619dcee3d2261ad2ee6bf12ffa06d98a0864d87602733ec86a64521f2b18177b200c",
    "bbe117577a615d6c770988c0bad946e208e24fa074e5ab3143db5bfce0fd108e4b82d120a93ad2caffffffffffffffff",
];
const N_LEN: usize = 384;
pub const USERNAME: &[u8] = b"Pair-Setup";

fn n() -> BigUint {
    BigUint::parse_bytes(N_HEX.concat().as_bytes(), 16).expect("constant is valid hex")
}

fn g() -> BigUint {
    BigUint::from(5u8)
}

fn pad(value: &BigUint) -> Vec<u8> {
    let bytes = value.to_bytes_be();
    let mut out = vec![0u8; N_LEN.saturating_sub(bytes.len())];
    out.extend_from_slice(&bytes);
    out
}

fn h(parts: &[&[u8]]) -> Vec<u8> {
    let mut hasher = Sha512::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().to_vec()
}

fn x(salt: &[u8], password: &[u8]) -> BigUint {
    let inner = h(&[USERNAME, b":", password]);
    BigUint::from_bytes_be(&h(&[salt, &inner]))
}

pub fn verifier(salt: &[u8], password: &[u8]) -> BigUint {
    g().modpow(&x(salt, password), &n())
}

/// The accessory's side of one pair-setup.
pub struct Server {
    salt: Vec<u8>,
    verifier: BigUint,
    b: BigUint,
    public_b: Vec<u8>,
}

pub struct Session {
    /// The shared session key; pair-setup derives its encryption and signing keys from it.
    pub key: Vec<u8>,
    /// The proof to send back (M2).
    pub server_proof: Vec<u8>,
}

impl Server {
    /// `secret` is 32 random bytes (`b`); `salt` is 16 random bytes.
    pub fn new(salt: &[u8], password: &[u8], secret: &[u8]) -> Self {
        let (n, g) = (n(), g());
        let verifier = verifier(salt, password);
        let b = BigUint::from_bytes_be(secret);
        let k = BigUint::from_bytes_be(&h(&[&pad(&n), &pad(&g)]));
        let public_b = pad(&((k * &verifier + g.modpow(&b, &n)) % &n));
        Self { salt: salt.to_vec(), verifier, b, public_b }
    }

    pub fn public_key(&self) -> &[u8] {
        &self.public_b
    }

    pub fn salt(&self) -> &[u8] {
        &self.salt
    }

    /// Check the controller's public key and proof. `None` means a wrong setup code (or a
    /// malformed key).
    pub fn verify(&self, public_a: &[u8], client_proof: &[u8]) -> Option<Session> {
        let n = n();
        let a = BigUint::from_bytes_be(public_a);
        if (&a % &n).is_zero() {
            return None;
        }
        let u = BigUint::from_bytes_be(&h(&[&pad(&a), &self.public_b]));
        let s = (&a * self.verifier.modpow(&u, &n)).modpow(&self.b, &n);
        let key = h(&[&pad(&s)]);
        let hn = h(&[&n.to_bytes_be()]);
        let hg = h(&[&g().to_bytes_be()]);
        let xor: Vec<u8> = hn.iter().zip(&hg).map(|(a, b)| a ^ b).collect();
        let expected = h(&[&xor, &h(&[USERNAME]), &self.salt, public_a, &self.public_b, &key]);
        if !constant_time_eq(&expected, client_proof) {
            return None;
        }
        let server_proof = h(&[public_a, &expected, &key]);
        Some(Session { key, server_proof })
    }
}

/// The controller's side, used by the tests and the pairing self-check.
pub struct Client {
    a: BigUint,
    pub public_a: Vec<u8>,
}

impl Client {
    pub fn new(secret: &[u8]) -> Self {
        let a = BigUint::from_bytes_be(secret);
        let public_a = pad(&g().modpow(&a, &n()));
        Self { a, public_a }
    }

    /// Returns (M1, K).
    pub fn proof(&self, salt: &[u8], password: &[u8], public_b: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let (n, g) = (n(), g());
        let b = BigUint::from_bytes_be(public_b);
        let k = BigUint::from_bytes_be(&h(&[&pad(&n), &pad(&g)]));
        let u = BigUint::from_bytes_be(&h(&[&self.public_a, &pad(&b)]));
        let x = x(salt, password);
        let gx = (k * g.modpow(&x, &n)) % &n;
        let base = ((&b + &n) - gx) % &n;
        let s = base.modpow(&(&self.a + u * x), &n);
        let key = h(&[&pad(&s)]);
        let hn = h(&[&n.to_bytes_be()]);
        let hg = h(&[&g.to_bytes_be()]);
        let xor: Vec<u8> = hn.iter().zip(&hg).map(|(a, b)| a ^ b).collect();
        let m1 = h(&[&xor, &h(&[USERNAME]), salt, &self.public_a, &pad(&b), &key]);
        (m1, key)
    }
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// Generated with hap-nodejs's fast-srp-hap (the SRP the original agent's bridge uses).
    const VECTORS: &str = include_str!("srp_vectors.json");

    fn hex(v: &Value, key: &str) -> Vec<u8> {
        hex::decode(v[key].as_str().unwrap()).unwrap()
    }

    #[test]
    fn matches_fast_srp_hap() {
        let v: Value = serde_json::from_str(VECTORS).unwrap();
        assert_eq!(N_HEX.concat(), v["N"].as_str().unwrap());
        let password = b"031-45-154";
        let salt = hex(&v, "salt");
        assert_eq!(pad(&verifier(&salt, password)), hex(&v, "v"));
        let server = Server::new(&salt, password, &hex(&v, "b"));
        assert_eq!(server.public_key(), &hex(&v, "B")[..]);
        let client = Client::new(&hex(&v, "a"));
        assert_eq!(client.public_a, hex(&v, "A"));
        let (m1, key) = client.proof(&salt, password, server.public_key());
        assert_eq!(m1, hex(&v, "M1"));
        assert_eq!(key, hex(&v, "K"));
        let session = server.verify(&client.public_a, &m1).expect("proof accepted");
        assert_eq!(session.key, hex(&v, "K"));
        assert_eq!(session.server_proof, hex(&v, "M2"));
    }

    #[test]
    fn wrong_code_is_refused() {
        let salt = [1u8; 16];
        let server = Server::new(&salt, b"111-22-333", &[9u8; 32]);
        let client = Client::new(&[3u8; 32]);
        let (m1, _) = client.proof(&salt, b"111-22-334", server.public_key());
        assert!(server.verify(&client.public_a, &m1).is_none());
    }
}
