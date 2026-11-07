import Foundation

/// One runnable thing a computer publishes: a command, or an automation (a routine).
struct CatalogEntry: Codable, Hashable, Identifiable {
    enum Kind: String, Codable { case command, automation }

    var kind: Kind
    /// Commands are identified by name, automations by their id.
    var id: String
    var name: String
    var icon: String = ""
    var description: String = ""
    var allowParams: Bool = false
    var hasOff: Bool = false
    var ground: String = "foreground"

    var key: String { "\(kind.rawValue):\(id)" }
    var isRoutine: Bool { kind == .automation }
    /// Asks something before running: parameters, or on/off.
    var asks: Bool { allowParams || hasOff }

    enum CodingKeys: String, CodingKey { case kind, id, name, icon, description, allowParams, hasOff, ground }

    init(kind: Kind, id: String, name: String, icon: String = "", description: String = "", allowParams: Bool = false, hasOff: Bool = false, ground: String = "foreground") {
        self.kind = kind
        self.id = id
        self.name = name
        self.icon = icon
        self.description = description
        self.allowParams = allowParams
        self.hasOff = hasOff
        self.ground = ground
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        kind = try c.decode(Kind.self, forKey: .kind)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        icon = try c.decodeIfPresent(String.self, forKey: .icon) ?? ""
        description = try c.decodeIfPresent(String.self, forKey: .description) ?? ""
        allowParams = try c.decodeIfPresent(Bool.self, forKey: .allowParams) ?? false
        hasOff = try c.decodeIfPresent(Bool.self, forKey: .hasOff) ?? false
        ground = try c.decodeIfPresent(String.self, forKey: .ground) ?? "foreground"
    }
}

/// A network interface of a paired computer, to send wake-up packets to.
struct NetInterface: Codable, Hashable {
    var name: String
    var mac: String
    var ip: String
    var primary: Bool
}

/// The server a computer is linked to, as the computer reports it.
struct ServerRef: Codable, Hashable {
    var url: String
    var computerId: String
}

/// A computer this phone paired with on the Wi-Fi. Its key is in the Keychain.
struct PairedComputer: Codable, Hashable, Identifiable {
    var id: String
    var name: String
    var hosts: [String]
    var port: Int
    var phoneId: String
    var pairedAt: Date
    var os: String?
    var interfaces: [NetInterface] = []
    var wakeForNetwork: Bool?
    var server: ServerRef?
    /// The last catalog seen, so tiles show while it sleeps.
    var catalog: [CatalogEntry] = []
    var lastSeen: Date?

    var keyAccount: String { "computer-key-\(id)" }
}

/// A computer as the server lists it.
struct ServerComputer: Codable, Hashable, Identifiable {
    var id: String
    var name: String
    var os: String
    var online: [String]
    var lastSeen: UInt64?
    var catalog: [CatalogEntry]
}

/// How the phone can reach a computer right now.
enum Reach: Equatable {
    case checking
    /// On the same Wi-Fi, talking to it directly.
    case home
    /// Through your server.
    case server
    /// Neither answers: asleep, off, or out of reach.
    case asleep

    var label: String {
        switch self {
        case .checking: "Checking…"
        case .home: "Home Wi-Fi"
        case .server: "Via server"
        case .asleep: "Asleep or offline"
        }
    }
}

/// One computer on the home screen: paired directly, known through the server, or both.
struct ComputerCard: Identifiable, Hashable {
    var id: String
    var name: String
    var os: String?
    var paired: PairedComputer?
    var server: ServerComputer?
    var reach: Reach
    var catalog: [CatalogEntry]

    static func == (a: ComputerCard, b: ComputerCard) -> Bool {
        a.id == b.id && a.name == b.name && a.reach == b.reach && a.catalog == b.catalog && a.paired == b.paired && a.server == b.server
    }

    func hash(into hasher: inout Hasher) { hasher.combine(id) }

    var routines: [CatalogEntry] { catalog.filter(\.isRoutine) }
    var commands: [CatalogEntry] { catalog.filter { !$0.isRoutine } }

    /// Whether this entry can run now, and how.
    func available(_ entry: CatalogEntry) -> Bool {
        switch reach {
        case .home: true
        case .server: server?.online.contains(entry.ground) ?? false
        case .checking, .asleep: false
        }
    }

    var symbol: String {
        switch os {
        case "windows": "pc"
        case "linux": "server.rack"
        default: "desktopcomputer"
        }
    }
}

/// A finished (or still running) run, from either path.
struct RunItem: Identifiable, Hashable {
    enum Status: String, Codable {
        case queued, running, succeeded, failed, offline
        case timedOut = "timed_out"
    }

    var id: String
    var at: Date
    var computer: String
    var name: String
    var params: String?
    var source: String
    var status: Status
    var exitCode: Int?
    var output: String

    var ok: Bool { status == .succeeded }
}

/// The computer's own history record.
struct RunRecord: Decodable {
    var id: String
    var at: UInt64
    var source: String
    var kind: CatalogEntry.Kind
    var target: String
    var name: String
    var params: String?
    var status: RunItem.Status
    var exitCode: Int?
    var output: String

    func item(computer: String) -> RunItem {
        RunItem(id: id, at: Date(timeIntervalSince1970: TimeInterval(at) / 1000), computer: computer, name: name,
                params: params, source: source, status: status, exitCode: exitCode, output: output)
    }
}

/// A run as the server records it.
struct ServerRun: Decodable {
    var id: String
    var computerName: String
    var targetName: String
    var params: String?
    var status: RunItem.Status
    var exitCode: Int?
    var output: String
    var source: String
    var createdAt: UInt64

    var item: RunItem {
        RunItem(id: id, at: Date(timeIntervalSince1970: TimeInterval(createdAt)), computer: computerName, name: targetName,
                params: params, source: "server/\(source)", status: status, exitCode: exitCode, output: output)
    }
}

/// Signed in to your CmdTrigger server; the API token is in the Keychain.
struct ServerAccount: Codable, Hashable {
    var url: String
    var email: String
}
