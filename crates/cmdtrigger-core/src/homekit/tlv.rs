//! TLV8, the pairing message format: type, length (≤255), value, with longer values split into
//! consecutive items of the same type.

pub const METHOD: u8 = 0x00;
pub const IDENTIFIER: u8 = 0x01;
pub const SALT: u8 = 0x02;
pub const PUBLIC_KEY: u8 = 0x03;
pub const PROOF: u8 = 0x04;
pub const ENCRYPTED_DATA: u8 = 0x05;
pub const STATE: u8 = 0x06;
pub const ERROR: u8 = 0x07;
pub const SIGNATURE: u8 = 0x0A;
pub const PERMISSIONS: u8 = 0x0B;
pub const FLAGS: u8 = 0x13;
pub const SEPARATOR: u8 = 0xFF;

pub const ERROR_UNKNOWN: u8 = 0x01;
pub const ERROR_AUTHENTICATION: u8 = 0x02;
pub const ERROR_MAX_PEERS: u8 = 0x04;
pub const ERROR_UNAVAILABLE: u8 = 0x06;

#[derive(Default, Debug, Clone)]
pub struct Tlv {
    items: Vec<(u8, Vec<u8>)>,
}

impl Tlv {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, kind: u8, value: impl AsRef<[u8]>) -> Self {
        self.items.push((kind, value.as_ref().to_vec()));
        self
    }

    pub fn push(&mut self, kind: u8, value: impl AsRef<[u8]>) {
        self.items.push((kind, value.as_ref().to_vec()));
    }

    pub fn state(state: u8) -> Self {
        Self::new().with(STATE, [state])
    }

    pub fn error(state: u8, error: u8) -> Self {
        Self::state(state).with(ERROR, [error])
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (kind, value) in &self.items {
            if value.is_empty() {
                out.extend([*kind, 0]);
                continue;
            }
            for chunk in value.chunks(255) {
                out.push(*kind);
                out.push(chunk.len() as u8);
                out.extend_from_slice(chunk);
            }
        }
        out
    }

    /// Decode, joining fragments. Items of the same type separated by another item stay
    /// separate (the pairing list uses separators).
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        let mut items: Vec<(u8, Vec<u8>)> = Vec::new();
        let mut i = 0;
        let mut last_full = false;
        while i < bytes.len() {
            if i + 2 > bytes.len() {
                return Err("truncated TLV header");
            }
            let (kind, len) = (bytes[i], bytes[i + 1] as usize);
            i += 2;
            if i + len > bytes.len() {
                return Err("truncated TLV value");
            }
            let value = &bytes[i..i + len];
            i += len;
            match items.last_mut() {
                Some((k, v)) if *k == kind && last_full => v.extend_from_slice(value),
                _ => items.push((kind, value.to_vec())),
            }
            last_full = len == 255;
        }
        Ok(Self { items })
    }

    pub fn get(&self, kind: u8) -> Option<&[u8]> {
        self.items.iter().find(|(k, _)| *k == kind).map(|(_, v)| v.as_slice())
    }

    pub fn byte(&self, kind: u8) -> Option<u8> {
        self.get(kind).and_then(|v| v.first().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_values_fragment_and_join() {
        let long = vec![7u8; 600];
        let encoded = Tlv::state(3).with(PUBLIC_KEY, &long).with(PROOF, [1, 2]).encode();
        assert_eq!(encoded.len(), 3 + (2 + 255) * 2 + (2 + 90) + 4);
        let decoded = Tlv::decode(&encoded).unwrap();
        assert_eq!(decoded.get(PUBLIC_KEY).unwrap(), &long[..]);
        assert_eq!(decoded.byte(STATE), Some(3));
        assert_eq!(decoded.get(PROOF).unwrap(), &[1, 2]);
    }
}
