import Foundation
import Observation
import SwiftUI

/// Everything the app shows and does. Computers paired on the Wi-Fi are reached directly; the
/// server (when signed in) covers being away from home and computers that aren't paired.
@MainActor
@Observable
final class AppModel {
    static let shared = AppModel()

    struct Toast: Identifiable, Equatable {
        let id = UUID()
        var text: String
        var ok: Bool
    }

    private(set) var paired: [PairedComputer] = [] { didSet { save() } }
    private(set) var account: ServerAccount? { didSet { save() } }
    private(set) var serverComputers: [ServerComputer] = []
    private(set) var serverError: String?
    /// Reach per paired computer id.
    private(set) var reach: [String: Reach] = [:]
    /// The address that answered last, per paired computer id.
    private var activeHost: [String: String] = [:]
    private(set) var runs: [RunItem] = []
    /// Entries running now, by `cardId|entry.key`.
    private(set) var busy: Set<String> = []
    private(set) var waking: Set<String> = []
    var toast: Toast?
    /// A finished run to show in full.
    var shownRun: RunItem?
    /// Set by a `cmdtrigger://pair` link (the QR code, opened from the Camera).
    var pairRequest: PhoneLink.Target?
    private(set) var refreshing = false

    private init() {
        let env = ProcessInfo.processInfo.environment
        if env["CT_RESET"] == "1" {
            for computer in Self.loadState()?.paired ?? [] { Keychain.delete(computer.keyAccount) }
            Keychain.delete(ServerAPI.tokenAccount)
            try? FileManager.default.removeItem(at: Self.stateURL)
            for key in ["phoneId", "intentItems"] { UserDefaults.standard.removeObject(forKey: key) }
        }
        let state = Self.loadState()
        paired = state?.paired ?? []
        account = state?.account
        for computer in paired { reach[computer.id] = .checking }
    }

    // MARK: What the home screen shows

    var cards: [ComputerCard] {
        var list: [ComputerCard] = paired.map { computer in
            let server = serverComputers.first { $0.id == computer.server?.computerId }
            var state = reach[computer.id] ?? .checking
            if state == .asleep, let server, !server.online.isEmpty { state = .server }
            let catalog = state == .server ? (server?.catalog ?? computer.catalog) : computer.catalog
            return ComputerCard(id: computer.id, name: computer.name, os: computer.os, paired: computer, server: server, reach: state, catalog: catalog)
        }
        let linked = Set(paired.compactMap { $0.server?.computerId })
        for server in serverComputers where !linked.contains(server.id) {
            list.append(ComputerCard(id: "server:\(server.id)", name: server.name, os: server.os, paired: nil, server: server,
                                     reach: server.online.isEmpty ? .asleep : .server, catalog: server.catalog))
        }
        return list
    }

    var isEmpty: Bool { paired.isEmpty && account == nil }

    // MARK: Refreshing

    func refresh() async {
        guard !refreshing else { return }
        refreshing = true
        defer { refreshing = false }
        async let direct: Void = refreshPaired()
        async let server: Void = refreshServer()
        _ = await (direct, server)
        await refreshRuns()
        IntentStore.save(cards)
    }

    private func refreshPaired() async {
        await withTaskGroup(of: Void.self) { group in
            for computer in paired {
                group.addTask { await self.refreshOne(computer.id) }
            }
        }
    }

    private func refreshOne(_ id: String) async {
        guard let computer = paired.first(where: { $0.id == id }) else { return }
        do {
            let status = try await call(computer, ["op": "status"])
            update(id) { c in
                c.name = status["name"] as? String ?? c.name
                c.os = status["os"] as? String ?? c.os
                if let catalog = Self.decode([CatalogEntry].self, status["catalog"]) { c.catalog = catalog }
                if let interfaces = Self.decode([NetInterface].self, status["interfaces"]) { c.interfaces = interfaces }
                c.wakeForNetwork = status["wakeForNetwork"] as? Bool
                c.server = Self.decode(ServerRef.self, status["server"])
                c.lastSeen = Date()
            }
            reach[id] = .home
        } catch PhoneLink.Failure.notPaired {
            reach[id] = .asleep
            show("\(computer.name) doesn't know this iPhone any more. Pair again in Settings.", ok: false)
        } catch {
            reach[id] = .asleep
        }
    }

    private func refreshServer() async {
        guard let account else {
            serverComputers = []
            return
        }
        do {
            serverComputers = try await ServerAPI.computers(account)
            serverError = nil
        } catch {
            serverError = error.localizedDescription
        }
    }

    func refreshRuns() async {
        var all: [RunItem] = []
        for computer in paired where reach[computer.id] == .home {
            if let reply = try? await call(computer, ["op": "runs", "limit": 50]),
               let records = Self.decode([RunRecord].self, reply["runs"]) {
                all += records.map { $0.item(computer: computer.name) }
            }
        }
        if let account, let server = try? await ServerAPI.runs(account) {
            let known = Set(all.map(\.id))
            all += server.filter { !known.contains($0.id) }
        }
        runs = all.sorted { $0.at > $1.at }
    }

    /// A sealed call to a paired computer: the address that worked last, then the others, then
    /// wherever Bonjour finds it now.
    private func call(_ computer: PairedComputer, _ request: [String: Any], timeout: TimeInterval = 8) async throws -> [String: Any] {
        if let host = activeHost[computer.id] {
            do {
                return try await PhoneLink.call(computer, host: host, request, timeout: timeout)
            } catch PhoneLink.Failure.unreachable {
                activeHost[computer.id] = nil
            }
        }
        var host = await PhoneLink.firstAnswering(computer.hosts, port: computer.port, id: computer.id)
        if host == nil, let found = await PhoneLink.discover().first(where: { $0.id == computer.id }) {
            update(computer.id) { c in
                c.hosts = found.hosts
                c.port = found.port
            }
            host = await PhoneLink.firstAnswering(found.hosts, port: found.port, id: computer.id)
        }
        guard let host, let current = paired.first(where: { $0.id == computer.id }) else { throw PhoneLink.Failure.unreachable }
        activeHost[computer.id] = host
        return try await PhoneLink.call(current, host: host, request, timeout: timeout)
    }

    // MARK: Running

    func isBusy(_ card: ComputerCard, _ entry: CatalogEntry) -> Bool { busy.contains("\(card.id)|\(entry.key)") }

    @discardableResult
    func run(_ entry: CatalogEntry, on card: ComputerCard, params: String? = nil) async -> RunItem? {
        let key = "\(card.id)|\(entry.key)"
        guard !busy.contains(key) else { return nil }
        busy.insert(key)
        defer { busy.remove(key) }
        let params = params?.trimmingCharacters(in: .whitespaces).isEmpty == false ? params : nil
        do {
            let item: RunItem
            if card.reach == .home, let computer = card.paired {
                var request: [String: Any] = ["op": "run", "kind": entry.kind.rawValue, "target": entry.id]
                if let params { request["params"] = params }
                let reply = try await call(computer, request, timeout: 65)
                if let record = Self.decode(RunRecord.self, reply["run"]) {
                    item = record.item(computer: card.name)
                } else {
                    show("\(entry.name) is still running on \(card.name).", ok: true)
                    return nil
                }
            } else if let account, let server = card.server, server.online.contains(entry.ground) {
                item = try await ServerAPI.run(account, computer: server.id, entry: entry, params: params)
            } else {
                throw PhoneLink.Failure.refused("\(card.name) is asleep or out of reach")
            }
            runs.insert(item, at: 0)
            let what = item.ok ? "done" : item.status == .failed ? "failed\(item.exitCode.map { " (exit \($0))" } ?? "")" : item.status.rawValue.replacingOccurrences(of: "_", with: " ")
            show("\(entry.name): \(what)", ok: item.ok)
            if !item.ok, !item.output.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { shownRun = item }
            return item
        } catch {
            show(error.localizedDescription, ok: false)
            if case PhoneLink.Failure.unreachable = error, let id = card.paired?.id { reach[id] = .asleep }
            return nil
        }
    }

    // MARK: Waking

    func isWaking(_ card: ComputerCard) -> Bool { waking.contains(card.id) }

    /// Awake: turn its screen on. Asleep: wake-up packets until it answers (up to 45 s), then
    /// the screen. Only works on the computer's own network.
    func wake(_ card: ComputerCard) async {
        guard let computer = card.paired, !waking.contains(card.id) else {
            show("Waking needs the computer paired with this iPhone on your Wi-Fi.", ok: false)
            return
        }
        waking.insert(card.id)
        defer { waking.remove(card.id) }
        if reach[computer.id] == .home, (try? await call(computer, ["op": "wakeScreen"])) != nil {
            show("\(card.name): screen on", ok: true)
            return
        }
        guard !computer.interfaces.isEmpty else {
            show("This iPhone hasn't learned \(card.name)'s network address yet. Open the app once while it's awake.", ok: false)
            return
        }
        guard Wake.sameNetwork(as: computer.interfaces) else {
            show("Waking works on the same Wi-Fi as \(card.name). You're on another network.", ok: false)
            return
        }
        let deadline = Date().addingTimeInterval(45)
        while Date() < deadline {
            await Wake.send(to: computer.interfaces)
            for host in computer.interfaces.map(\.ip) { Wake.knock(host: host, port: computer.port) }
            if await PhoneLink.firstAnswering(computer.interfaces.map(\.ip) + computer.hosts, port: computer.port, id: computer.id, timeout: 2.5) != nil {
                await refreshOne(computer.id)
                _ = try? await call(computer, ["op": "wakeScreen"])
                show("\(card.name) is awake", ok: true)
                return
            }
            try? await Task.sleep(for: .seconds(1))
        }
        var hint = "\(card.name) didn't wake up."
        if computer.wakeForNetwork == false {
            hint += " Turn on Wake for network access on it (CmdTrigger → Settings → iPhone)."
        } else {
            hint += " It may be shut down (only a sleeping computer can be woken), or on battery."
        }
        show(hint, ok: false)
    }

    // MARK: Pairing and accounts

    func pair(_ target: PhoneLink.Target, code: String) async throws {
        let computer = try await PhoneLink.pair(target, code: code, phoneName: Self.phoneName)
        if let index = paired.firstIndex(where: { $0.id == computer.id }) {
            paired[index] = computer
        } else {
            paired.append(computer)
        }
        reach[computer.id] = .checking
        activeHost[computer.id] = computer.hosts.first
        show("Paired with \(computer.name)", ok: true)
        await refreshOne(computer.id)
        IntentStore.save(cards)
    }

    func unpair(_ computer: PairedComputer) async {
        _ = try? await call(computer, ["op": "unpair"], timeout: 4)
        Keychain.delete(computer.keyAccount)
        paired.removeAll { $0.id == computer.id }
        reach[computer.id] = nil
        IntentStore.save(cards)
    }

    func signIn(url: String, email: String, password: String) async throws {
        account = try await ServerAPI.signIn(url: url, email: email, password: password, phoneName: "\(Self.phoneName) (CmdTrigger app)")
        await refreshServer()
        IntentStore.save(cards)
    }

    func signOut() {
        ServerAPI.signOut()
        account = nil
        serverComputers = []
        IntentStore.save(cards)
    }

    /// `cmdtrigger://pair?…`
    func open(_ url: URL) {
        if let target = PhoneLink.Target(url: url) { pairRequest = target }
    }

    func show(_ text: String, ok: Bool) {
        toast = Toast(text: text, ok: ok)
    }

    // MARK: Storage

    static var phoneName: String { UIDevice.current.name }

    private func update(_ id: String, _ change: (inout PairedComputer) -> Void) {
        guard let index = paired.firstIndex(where: { $0.id == id }) else { return }
        var copy = paired[index]
        change(&copy)
        if copy != paired[index] { paired[index] = copy }
    }

    /// The paired computers and the server account, in one file written at once (not
    /// UserDefaults, which can lose a write made just before the app is killed). Readable after
    /// the first unlock, so Siri can run things while the phone is locked.
    private struct State: Codable {
        var paired: [PairedComputer]
        var account: ServerAccount?
    }

    private static var stateURL: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("state.json")
    }

    private func save() {
        guard let data = try? JSONEncoder().encode(State(paired: paired, account: account)) else { return }
        try? FileManager.default.createDirectory(at: Self.stateURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? data.write(to: Self.stateURL, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    private static func loadState() -> State? {
        (try? Data(contentsOf: stateURL)).flatMap { try? JSONDecoder().decode(State.self, from: $0) }
    }

    /// A JSON value from `JSONSerialization` → a `Decodable`.
    static func decode<T: Decodable>(_ type: T.Type, _ value: Any?) -> T? {
        guard let value, !(value is NSNull), JSONSerialization.isValidJSONObject(value) || value is String || value is NSNumber,
              let data = try? JSONSerialization.data(withJSONObject: value, options: [.fragmentsAllowed]) else { return nil }
        return try? JSONDecoder().decode(T.self, from: data)
    }
}
