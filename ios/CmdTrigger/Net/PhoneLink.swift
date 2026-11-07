import CryptoKit
import Foundation

/// Talking to a computer directly on the Wi-Fi (`crates/cmdtrigger-core/src/phone/mod.rs`).
enum PhoneLink {
    static let serviceType = "_cmdtrigger._tcp"
    static let defaultPort = 7180

    enum Failure: LocalizedError {
        case unreachable
        case notPaired
        case refused(String)
        case badReply

        var errorDescription: String? {
            switch self {
            case .unreachable: "Can't reach the computer on this Wi-Fi."
            case .notPaired: "The computer doesn't know this iPhone any more. Pair again."
            case let .refused(message): message.prefix(1).uppercased() + message.dropFirst()
            case .badReply: "The computer's answer couldn't be read. Pair again."
            }
        }
    }

    /// Where to pair: from a QR code / link, a computer found on the Wi-Fi, or a typed address.
    struct Target: Hashable, Identifiable {
        var id: String?
        var name: String
        var hosts: [String]
        var port: Int
        var code: String?

        var key: String { id ?? "\(hosts.first ?? ""):\(port)" }
        var identifier: String { key }

        /// `cmdtrigger://pair?v=1&id=…&n=…&h=ip1,ip2&p=7180&c=CODE`
        init?(url: URL) {
            guard url.scheme == "cmdtrigger", url.host == "pair",
                  let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems else { return nil }
            let value = { (name: String) in items.first(where: { $0.name == name })?.value }
            let hosts = (value("h") ?? "").split(separator: ",").map(String.init).filter { !$0.isEmpty }
            guard !hosts.isEmpty || value("id") != nil else { return nil }
            id = value("id")
            name = value("n") ?? "Computer"
            self.hosts = hosts
            port = value("p").flatMap(Int.init) ?? PhoneLink.defaultPort
            code = value("c").flatMap(PhoneCrypto.normalize)
        }

        init(id: String?, name: String, hosts: [String], port: Int, code: String? = nil) {
            self.id = id
            self.name = name
            self.hosts = hosts
            self.port = port
            self.code = code
        }

        /// "192.168.3.10" or "192.168.3.10:7180", as typed.
        init?(address: String) {
            let parts = address.trimmingCharacters(in: .whitespaces).split(separator: ":")
            guard let host = parts.first, !host.isEmpty else { return nil }
            let port = parts.count > 1 ? Int(parts[1]) : PhoneLink.defaultPort
            guard let port else { return nil }
            self.init(id: nil, name: String(host), hosts: [String(host)], port: port)
        }
    }

    struct Hello: Decodable {
        var app: String
        var id: String
        var name: String
        var pairing: Bool
    }

    /// The phone's own id, the same for every computer it pairs with.
    static var phoneId: String {
        if let id = UserDefaults.standard.string(forKey: "phoneId") { return id }
        let id = UUID().uuidString
        UserDefaults.standard.set(id, forKey: "phoneId")
        return id
    }

    // MARK: Finding computers

    /// Computers advertising CmdTrigger on this Wi-Fi.
    static func discover(seconds: TimeInterval = 2.5) async -> [Target] {
        await Bonjour.browse(serviceType, for: seconds).compactMap { service in
            let hosts = (service.txt["ip"] ?? "").split(separator: ",").map(String.init).filter { !$0.isEmpty }
            guard !hosts.isEmpty, let port = service.txt["port"].flatMap(Int.init) else { return nil }
            return Target(id: service.txt["id"], name: service.name, hosts: hosts, port: port)
        }
    }

    static func hello(host: String, port: Int, timeout: TimeInterval = 2.5) async -> Hello? {
        guard let url = URL(string: "http://\(host):\(port)/phone/v1/hello") else { return nil }
        var request = URLRequest(url: url)
        request.timeoutInterval = timeout
        guard let (data, response) = try? await session.data(for: request),
              (response as? HTTPURLResponse)?.statusCode == 200,
              let hello = try? JSONDecoder().decode(Hello.self, from: data), hello.app == "cmdtrigger" else { return nil }
        return hello
    }

    /// The first of `hosts` that answers as this computer (all tried at once).
    static func firstAnswering(_ hosts: [String], port: Int, id: String?, timeout: TimeInterval = 2.5) async -> String? {
        await withTaskGroup(of: (String, Hello?).self) { group in
            for host in hosts {
                group.addTask { (host, await hello(host: host, port: port, timeout: timeout)) }
            }
            for await (host, hello) in group {
                if let hello, id == nil || hello.id == id {
                    group.cancelAll()
                    return host
                }
            }
            return nil
        }
    }

    // MARK: Pairing

    static func pair(_ target: Target, code typed: String, phoneName: String) async throws -> PairedComputer {
        guard let code = PhoneCrypto.normalize(typed) else {
            throw Failure.refused("the code is 16 letters and digits, like 7K3Q-H2MZ-P9WC-4T8B")
        }
        guard let host = await firstAnswering(target.hosts, port: target.port, id: target.id) else { throw Failure.unreachable }
        let phoneId = self.phoneId
        let codeKey = PhoneCrypto.codeKey(code)
        let secret = Curve25519.KeyAgreement.PrivateKey()
        let phonePublic = secret.publicKey.rawRepresentation
        let body: [String: String] = [
            "phoneId": phoneId,
            "phoneName": phoneName,
            "phonePublic": phonePublic.base64EncodedString(),
            "proof": PhoneCrypto.phoneProof(codeKey: codeKey, phonePublic: phonePublic, phoneId: phoneId).base64EncodedString(),
        ]
        let (status, json) = try await post(host: host, port: target.port, path: "/phone/v1/pair", body: body, timeout: 10)
        guard status == 200 else {
            let message = json["error"] as? String ?? "the computer refused"
            throw Failure.refused(message == "wrong code" ? "that code doesn't match the one on the computer" : message)
        }
        guard let macId = json["macId"] as? String,
              let macPublic = (json["macPublic"] as? String).flatMap({ Data(base64Encoded: $0) }),
              let proof = (json["proof"] as? String).flatMap({ Data(base64Encoded: $0) }),
              let macKey = try? Curve25519.KeyAgreement.PublicKey(rawRepresentation: macPublic) else { throw Failure.badReply }
        // The computer proves it knows the code too: nobody in between is pretending to be it.
        guard proof == PhoneCrypto.macProof(codeKey: codeKey, macPublic: macPublic, phonePublic: phonePublic) else {
            throw Failure.refused("the computer couldn't prove it showed that code")
        }
        let shared = try secret.sharedSecretFromKeyAgreement(with: macKey)
        let key = PhoneCrypto.deriveKey(codeKey: codeKey, shared: shared, phonePublic: phonePublic, macPublic: macPublic)
        let computer = PairedComputer(
            id: macId, name: json["macName"] as? String ?? target.name,
            hosts: [host] + target.hosts.filter { $0 != host }, port: target.port, phoneId: phoneId, pairedAt: Date()
        )
        Keychain.set(key.withUnsafeBytes { Data($0) }.base64EncodedString(), for: computer.keyAccount)
        return computer
    }

    // MARK: Sealed calls

    static func key(for computer: PairedComputer) -> SymmetricKey? {
        guard let text = Keychain.get(computer.keyAccount), let data = Data(base64Encoded: text), data.count == 32 else { return nil }
        return SymmetricKey(data: data)
    }

    /// One request; returns the opened reply. Throws `.unreachable` when no saved address
    /// answers (the caller may look for the computer again).
    static func call(_ computer: PairedComputer, host: String, _ request: [String: Any], timeout: TimeInterval = 8) async throws -> [String: Any] {
        guard let key = key(for: computer) else { throw Failure.notPaired }
        var request = request
        request["at"] = Int(Date().timeIntervalSince1970)
        let plain = try JSONSerialization.data(withJSONObject: request)
        let sealed = try PhoneCrypto.seal(plain, key: key, direction: "call", phoneId: computer.phoneId)
        let (status, json) = try await post(
            host: host, port: computer.port, path: "/phone/v1/call",
            body: ["phone": computer.phoneId, "sealed": sealed.base64EncodedString()], timeout: timeout
        )
        if status == 401 { throw Failure.notPaired }
        guard status == 200 else { throw Failure.refused(json["error"] as? String ?? "the computer refused (\(status))") }
        guard let text = json["sealed"] as? String, let combined = Data(base64Encoded: text),
              let opened = try? PhoneCrypto.open(combined, key: key, direction: "reply", phoneId: computer.phoneId),
              let reply = try JSONSerialization.jsonObject(with: opened) as? [String: Any] else { throw Failure.badReply }
        if let error = reply["error"] as? String { throw Failure.refused(error) }
        return reply
    }

    // MARK: HTTP

    /// Short timeouts, no caching, no cookies: these are LAN calls.
    static let session: URLSession = {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 10
        config.waitsForConnectivity = false
        return URLSession(configuration: config)
    }()

    private static func post(host: String, port: Int, path: String, body: [String: String], timeout: TimeInterval) async throws -> (Int, [String: Any]) {
        guard let url = URL(string: "http://\(host):\(port)\(path)") else { throw Failure.unreachable }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.timeoutInterval = timeout
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await session.data(for: request)
        } catch {
            throw Failure.unreachable
        }
        let json = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
        return ((response as? HTTPURLResponse)?.statusCode ?? 0, json)
    }
}
