import CryptoKit
import Foundation

/// The phone link's cryptography; the computer's side is `crates/cmdtrigger-core/src/phone/crypto.rs`
/// and both are checked against the same vectors (`CmdTriggerTests/phone_vectors.json`).
///
/// Pairing proves the one-time code (16 Crockford base-32 characters, 80 bits) without sending
/// it, in both directions, then derives a per-computer key from an X25519 exchange. Every call
/// after that is sealed with ChaCha20-Poly1305.
enum PhoneCrypto {
    static let codeLength = 16
    private static let alphabet = Set("0123456789ABCDEFGHJKMNPQRSTVWXYZ")

    /// What the user typed (or the QR code carried) → the canonical code: no dashes or spaces,
    /// upper case, O→0 and I/L→1. `nil` unless it's 16 valid characters.
    static func normalize(_ typed: String) -> String? {
        let code = String(typed.uppercased().compactMap { c -> Character? in
            if c.isWhitespace || c == "-" { return nil }
            switch c {
            case "O": return "0"
            case "I", "L": return "1"
            default: return c
            }
        })
        guard code.count == codeLength, code.allSatisfy({ alphabet.contains($0) }) else { return nil }
        return code
    }

    /// "ABCD-EFGH-JKMN-PQRS".
    static func grouped(_ code: String) -> String {
        stride(from: 0, to: code.count, by: 4).map { i in
            let start = code.index(code.startIndex, offsetBy: i)
            return String(code[start ..< code.index(start, offsetBy: min(4, code.count - i))])
        }.joined(separator: "-")
    }

    static func codeKey(_ code: String) -> SymmetricKey {
        SymmetricKey(data: SHA256.hash(data: Data("cmdtrigger-pair-v1:\(code)".utf8)))
    }

    private static func hmac(_ key: SymmetricKey, _ parts: [Data]) -> Data {
        var mac = HMAC<SHA256>(key: key)
        for part in parts { mac.update(data: part) }
        return Data(mac.finalize())
    }

    static func phoneProof(codeKey: SymmetricKey, phonePublic: Data, phoneId: String) -> Data {
        hmac(codeKey, [Data("phone".utf8), phonePublic, Data(phoneId.utf8)])
    }

    static func macProof(codeKey: SymmetricKey, macPublic: Data, phonePublic: Data) -> Data {
        hmac(codeKey, [Data("mac".utf8), macPublic, phonePublic])
    }

    static func deriveKey(codeKey: SymmetricKey, shared: SharedSecret, phonePublic: Data, macPublic: Data) -> SymmetricKey {
        shared.hkdfDerivedSymmetricKey(
            using: SHA256.self,
            salt: codeKey.withUnsafeBytes { Data($0) },
            sharedInfo: Data("cmdtrigger-phone-v1".utf8) + phonePublic + macPublic,
            outputByteCount: 32
        )
    }

    private static func aad(_ direction: String, _ phoneId: String) -> Data {
        Data("cmdtrigger-phone-v1:\(direction):\(phoneId)".utf8)
    }

    /// `nonce ‖ ciphertext ‖ tag`.
    static func seal(_ plaintext: Data, key: SymmetricKey, direction: String, phoneId: String, nonce: ChaChaPoly.Nonce = ChaChaPoly.Nonce()) throws -> Data {
        try ChaChaPoly.seal(plaintext, using: key, nonce: nonce, authenticating: aad(direction, phoneId)).combined
    }

    static func open(_ combined: Data, key: SymmetricKey, direction: String, phoneId: String) throws -> Data {
        try ChaChaPoly.open(ChaChaPoly.SealedBox(combined: combined), using: key, authenticating: aad(direction, phoneId))
    }
}

extension Data {
    init?(hex: String) {
        var bytes: [UInt8] = []
        var index = hex.startIndex
        while index < hex.endIndex {
            let next = hex.index(index, offsetBy: 2, limitedBy: hex.endIndex) ?? hex.endIndex
            guard let byte = UInt8(hex[index ..< next], radix: 16) else { return nil }
            bytes.append(byte)
            index = next
        }
        self.init(bytes)
    }

    var hex: String { map { String(format: "%02x", $0) }.joined() }
}
