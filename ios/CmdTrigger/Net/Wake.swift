import Foundation
import Network

/// Waking a sleeping computer from the phone, over Wi-Fi.
///
/// iOS won't let this app broadcast (that needs Apple's multicast entitlement, which a free team
/// can't get). So the magic packet goes straight to each of the computer's addresses instead:
/// a sleeping Mac with "Wake for network access" keeps its network card listening and wakes
/// on it. It also tries to connect to the computer, which wakes it through a Bonjour Sleep
/// Proxy (an Apple TV or HomePod) when the home has one.
enum Wake {
    /// Six 0xFF bytes, then the MAC sixteen times.
    static func packet(mac: String) -> Data? {
        let parts = mac.split(whereSeparator: { $0 == ":" || $0 == "-" })
        guard parts.count == 6 else { return nil }
        var bytes: [UInt8] = []
        for part in parts {
            guard part.count <= 2, let byte = UInt8(part, radix: 16) else { return nil }
            bytes.append(byte)
        }
        return Data(repeating: 0xFF, count: 6) + Data((0 ..< 16).flatMap { _ in bytes })
    }

    /// Whether the phone is on the same network as one of these addresses.
    static func sameNetwork(as interfaces: [NetInterface]) -> Bool {
        guard let (ip, prefix) = LocalNetwork.wifi() else { return false }
        return interfaces.contains { inSubnet($0.ip, of: ip, prefix: prefix) }
    }

    static func inSubnet(_ address: String, of own: String, prefix: Int) -> Bool {
        var a = in_addr(), b = in_addr()
        guard inet_pton(AF_INET, address, &a) == 1, inet_pton(AF_INET, own, &b) == 1, (1 ... 32).contains(prefix) else { return false }
        let mask = UInt32.max << (32 - UInt32(prefix))
        return UInt32(bigEndian: a.s_addr) & mask == UInt32(bigEndian: b.s_addr) & mask
    }

    /// One round of wake-up packets to every interface. True if any left the phone.
    @discardableResult
    static func send(to interfaces: [NetInterface]) async -> Bool {
        await blocking {
            guard let socket = UDPSocket() else { return false }
            socket.allowBroadcast()
            var sent = false
            for interface in interfaces {
                guard let packet = packet(mac: interface.mac) else { continue }
                var targets = [interface.ip]
                if let broadcast = LocalNetwork.broadcast() { targets.append(broadcast) }
                for target in targets {
                    for port: UInt16 in [9, 7] {
                        sent = socket.send(packet, to: target, port: port) || sent
                    }
                }
            }
            return sent
        }
    }

    /// Opens (and drops) a TCP connection, so a sleep proxy wakes the computer for it.
    static func knock(host: String, port: Int) {
        guard let port = NWEndpoint.Port(rawValue: UInt16(clamping: port)) else { return }
        let connection = NWConnection(host: NWEndpoint.Host(host), port: port, using: .tcp)
        connection.start(queue: .global(qos: .utility))
        DispatchQueue.global().asyncAfter(deadline: .now() + 2) { connection.cancel() }
    }
}
