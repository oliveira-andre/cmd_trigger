import CryptoKit
import XCTest
@testable import CmdTrigger

/// The phone link's crypto against the vectors the Rust side writes
/// (`crates/cmdtrigger-core/src/phone/crypto.rs`), so both ends agree byte for byte.
final class PhoneCryptoTests: XCTestCase {
    private var v: [String: String] = [:]

    override func setUpWithError() throws {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "phone_vectors", withExtension: "json"))
        v = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: String])
    }

    private func hex(_ key: String) throws -> Data { try XCTUnwrap(Data(hex: XCTUnwrap(v[key]))) }

    func testCodesNormalize() {
        XCTAssertEqual(PhoneCrypto.normalize("7k3q-h2mz-p9wc-4t8b"), "7K3QH2MZP9WC4T8B")
        XCTAssertEqual(PhoneCrypto.normalize("OIL0 0000 0000 0000"), "0110000000000000")
        XCTAssertNil(PhoneCrypto.normalize("7K3Q-H2MZ"))
        XCTAssertNil(PhoneCrypto.normalize("UUUU-UUUU-UUUU-UUUU"))
        XCTAssertEqual(PhoneCrypto.grouped("7K3QH2MZP9WC4T8B"), "7K3Q-H2MZ-P9WC-4T8B")
    }

    func testMatchesTheComputer() throws {
        let code = try XCTUnwrap(v["code"])
        let codeKey = PhoneCrypto.codeKey(code)
        XCTAssertEqual(codeKey.withUnsafeBytes { Data($0) }, try hex("codeKey"))

        let phone = try Curve25519.KeyAgreement.PrivateKey(rawRepresentation: hex("phoneSecret"))
        let phonePublic = phone.publicKey.rawRepresentation
        XCTAssertEqual(phonePublic, try hex("phonePublic"))
        let phoneId = try XCTUnwrap(v["phoneId"])
        XCTAssertEqual(PhoneCrypto.phoneProof(codeKey: codeKey, phonePublic: phonePublic, phoneId: phoneId), try hex("phoneProof"))

        let macPublic = try hex("macPublic")
        XCTAssertEqual(PhoneCrypto.macProof(codeKey: codeKey, macPublic: macPublic, phonePublic: phonePublic), try hex("macProof"))
        let shared = try phone.sharedSecretFromKeyAgreement(with: Curve25519.KeyAgreement.PublicKey(rawRepresentation: macPublic))
        let key = PhoneCrypto.deriveKey(codeKey: codeKey, shared: shared, phonePublic: phonePublic, macPublic: macPublic)
        XCTAssertEqual(key.withUnsafeBytes { Data($0) }, try hex("key"))

        let plaintext = Data(try XCTUnwrap(v["plaintext"]).utf8)
        let sealed = try PhoneCrypto.seal(plaintext, key: key, direction: "call", phoneId: phoneId, nonce: ChaChaPoly.Nonce(data: hex("nonce")))
        XCTAssertEqual(sealed, try hex("sealedCall"))
        XCTAssertEqual(try PhoneCrypto.open(sealed, key: key, direction: "call", phoneId: phoneId), plaintext)
        XCTAssertThrowsError(try PhoneCrypto.open(sealed, key: key, direction: "reply", phoneId: phoneId))
    }

    func testPairingLinks() throws {
        let url = try XCTUnwrap(URL(string: "cmdtrigger://pair?v=1&id=ab12&n=Andr%C3%A9%E2%80%99s%20Mac&h=192.168.3.10,10.0.0.5&p=7180&c=7K3QH2MZP9WC4T8B"))
        let target = try XCTUnwrap(PhoneLink.Target(url: url))
        XCTAssertEqual(target.id, "ab12")
        XCTAssertEqual(target.name, "André’s Mac")
        XCTAssertEqual(target.hosts, ["192.168.3.10", "10.0.0.5"])
        XCTAssertEqual(target.port, 7180)
        XCTAssertEqual(target.code, "7K3QH2MZP9WC4T8B")
        XCTAssertNil(PhoneLink.Target(url: try XCTUnwrap(URL(string: "https://example.com/pair"))))
        XCTAssertEqual(PhoneLink.Target(address: "192.168.1.20:7300")?.port, 7300)
    }

    func testWakePackets() throws {
        let packet = try XCTUnwrap(Wake.packet(mac: "b0:fe:e5:25:cb:c3"))
        XCTAssertEqual(packet.count, 102)
        XCTAssertEqual(Array(packet.prefix(6)), [UInt8](repeating: 0xFF, count: 6))
        XCTAssertEqual(Array(packet[6 ..< 12]), [0xB0, 0xFE, 0xE5, 0x25, 0xCB, 0xC3])
        XCTAssertNil(Wake.packet(mac: "nope"))
        XCTAssertTrue(Wake.inSubnet("192.168.3.10", of: "192.168.3.77", prefix: 24))
        XCTAssertFalse(Wake.inSubnet("192.168.4.10", of: "192.168.3.77", prefix: 24))
    }

    func testServerAddresses() {
        XCTAssertEqual(ServerAPI.normalize("cmd.example.com/"), "https://cmd.example.com")
        XCTAssertEqual(ServerAPI.normalize("192.168.3.10:8080"), "http://192.168.3.10:8080")
        XCTAssertEqual(ServerAPI.normalize("http://mac.local:8080"), "http://mac.local:8080")
        XCTAssertNil(ServerAPI.normalize("  "))
    }
}
