import Foundation

/// Your CmdTrigger server (`crates/cmdtrigger-server`), for when you're away from home. The
/// phone signs in once and keeps an API token, not the password.
enum ServerAPI {
    static let tokenAccount = "server-token"

    enum Failure: LocalizedError {
        case message(String)
        case unreachable

        var errorDescription: String? {
            switch self {
            case let .message(m): m.prefix(1).uppercased() + m.dropFirst()
            case .unreachable: "Can't reach your server."
            }
        }
    }

    /// "cmd.example.com" → https; IPs, `localhost` and `.local` names → http.
    static func normalize(_ typed: String) -> String? {
        var text = typed.trimmingCharacters(in: .whitespacesAndNewlines)
        while text.hasSuffix("/") { text.removeLast() }
        guard !text.isEmpty else { return nil }
        if !text.contains("://") {
            let host = text.split(separator: ":").first.map(String.init) ?? text
            let local = host == "localhost" || host.hasSuffix(".local") || host.allSatisfy { $0.isNumber || $0 == "." }
            text = (local ? "http://" : "https://") + text
        }
        return URL(string: text)?.host == nil ? nil : text
    }

    static var token: String? { Keychain.get(tokenAccount) }

    /// Signs in, creates an API token for this phone, signs the session out again.
    static func signIn(url typed: String, email: String, password: String, phoneName: String) async throws -> ServerAccount {
        guard let url = normalize(typed) else { throw Failure.message("enter your server's address") }
        let (status, session) = try await request(url, "POST", "/api/v1/login", body: ["email": email, "password": password], token: nil)
        guard status == 200, let sessionToken = session["token"] as? String else {
            throw Failure.message(session["error"] as? String ?? "sign-in failed (\(status))")
        }
        defer { Task { _ = try? await request(url, "POST", "/api/v1/logout", body: [:], token: sessionToken) } }
        let (created, token) = try await request(url, "POST", "/api/v1/tokens", body: ["name": phoneName], token: sessionToken)
        guard created == 200, let secret = token["token"] as? String else {
            throw Failure.message(token["error"] as? String ?? "couldn't create a token for this iPhone")
        }
        Keychain.set(secret, for: tokenAccount)
        return ServerAccount(url: url, email: email)
    }

    static func signOut() {
        Keychain.delete(tokenAccount)
    }

    static func computers(_ account: ServerAccount) async throws -> [ServerComputer] {
        try await decode([ServerComputer].self, account, "GET", "/api/v1/computers", body: nil)
    }

    static func run(_ account: ServerAccount, computer: String, entry: CatalogEntry, params: String?) async throws -> RunItem {
        var body: [String: Any] = ["computer": computer, "target": entry.id, "kind": entry.kind.rawValue, "wait": 50]
        if let params { body["params"] = params }
        return try await decode(ServerRun.self, account, "POST", "/api/v1/run", body: body, timeout: 65).item
    }

    static func runs(_ account: ServerAccount, limit: Int = 50) async throws -> [RunItem] {
        try await decode([ServerRun].self, account, "GET", "/api/v1/runs?limit=\(limit)", body: nil).map(\.item)
    }

    // MARK: HTTP

    private static func decode<T: Decodable>(_ type: T.Type, _ account: ServerAccount, _ method: String, _ path: String, body: [String: Any]?, timeout: TimeInterval = 15) async throws -> T {
        guard let token else { throw Failure.message("sign in to your server again") }
        let (status, data) = try await raw(account.url, method, path, body: body, token: token, timeout: timeout)
        guard (200 ..< 300).contains(status) else {
            let json = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
            throw Failure.message(json?["error"] as? String ?? "the server answered \(status)")
        }
        do {
            return try JSONDecoder().decode(T.self, from: data)
        } catch {
            throw Failure.message("the server's answer couldn't be read")
        }
    }

    private static func request(_ base: String, _ method: String, _ path: String, body: [String: Any]?, token: String?) async throws -> (Int, [String: Any]) {
        let (status, data) = try await raw(base, method, path, body: body, token: token, timeout: 15)
        return (status, (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:])
    }

    private static func raw(_ base: String, _ method: String, _ path: String, body: [String: Any]?, token: String?, timeout: TimeInterval) async throws -> (Int, Data) {
        guard let url = URL(string: base + path) else { throw Failure.unreachable }
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.timeoutInterval = timeout
        if let token { request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization") }
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
        }
        do {
            let (data, response) = try await PhoneLink.session.data(for: request)
            return ((response as? HTTPURLResponse)?.statusCode ?? 0, data)
        } catch {
            throw Failure.unreachable
        }
    }
}
