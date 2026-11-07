import AppIntents
import Foundation

/// What Siri and Shortcuts can pick from: the last commands, routines and computers the app saw.
enum IntentStore {
    struct Item: Codable, Hashable {
        var cardId: String
        var computer: String
        var entry: CatalogEntry
        var id: String { "\(cardId)|\(entry.key)" }
    }

    struct Saved: Codable {
        var items: [Item]
        var computers: [String: String]
    }

    @MainActor
    static func save(_ cards: [ComputerCard]) {
        let saved = Saved(
            items: cards.flatMap { card in card.catalog.map { Item(cardId: card.id, computer: card.name, entry: $0) } },
            computers: Dictionary(cards.filter { $0.paired != nil }.map { ($0.id, $0.name) }, uniquingKeysWith: { a, _ in a })
        )
        if let data = try? JSONEncoder().encode(saved) { UserDefaults.standard.set(data, forKey: "intentItems") }
        CmdTriggerShortcuts.updateAppShortcutParameters()
    }

    static func load() -> Saved {
        UserDefaults.standard.data(forKey: "intentItems").flatMap { try? JSONDecoder().decode(Saved.self, from: $0) }
            ?? Saved(items: [], computers: [:])
    }
}

struct ItemEntity: AppEntity {
    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Command or routine"
    static let defaultQuery = ItemQuery()

    var id: String
    var name: String
    var computer: String
    var routine: Bool

    var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(name)", subtitle: "\(routine ? "Routine" : "Command") on \(computer)")
    }

    init(_ item: IntentStore.Item) {
        id = item.id
        name = item.entry.name
        computer = item.computer
        routine = item.entry.isRoutine
    }
}

struct ItemQuery: EntityStringQuery {
    func entities(for identifiers: [String]) async throws -> [ItemEntity] {
        IntentStore.load().items.filter { identifiers.contains($0.id) }.map(ItemEntity.init)
    }

    func entities(matching string: String) async throws -> [ItemEntity] {
        IntentStore.load().items.filter { $0.entry.name.localizedCaseInsensitiveContains(string) }.map(ItemEntity.init)
    }

    func suggestedEntities() async throws -> [ItemEntity] {
        IntentStore.load().items.map(ItemEntity.init)
    }
}

struct ComputerEntity: AppEntity {
    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Computer"
    static let defaultQuery = ComputerQuery()

    var id: String
    var name: String

    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(name)") }
}

struct ComputerQuery: EntityStringQuery {
    func entities(for identifiers: [String]) async throws -> [ComputerEntity] {
        IntentStore.load().computers.filter { identifiers.contains($0.key) }.map { ComputerEntity(id: $0.key, name: $0.value) }
    }

    func entities(matching string: String) async throws -> [ComputerEntity] {
        IntentStore.load().computers.filter { $0.value.localizedCaseInsensitiveContains(string) }.map { ComputerEntity(id: $0.key, name: $0.value) }
    }

    func suggestedEntities() async throws -> [ComputerEntity] {
        IntentStore.load().computers.map { ComputerEntity(id: $0.key, name: $0.value) }.sorted { $0.name < $1.name }
    }
}

struct RunItemIntent: AppIntent {
    static let title: LocalizedStringResource = "Run a Command or Routine"
    static let description = IntentDescription("Runs one of your computers' commands or routines with CmdTrigger.")

    @Parameter(title: "Command or routine")
    var item: ItemEntity

    @Parameter(title: "Parameters", description: "For commands that take parameters, or \"on\"/\"off\".")
    var params: String?

    static var parameterSummary: some ParameterSummary {
        Summary("Run \(\.$item)") { \.$params }
    }

    @MainActor
    func perform() async throws -> some IntentResult & ProvidesDialog {
        let model = AppModel.shared
        await model.refresh()
        guard let card = model.cards.first(where: { card in card.catalog.contains { "\(card.id)|\($0.key)" == item.id } }),
              let entry = card.catalog.first(where: { "\(card.id)|\($0.key)" == item.id }) else {
            return .result(dialog: "I can't find \(item.name) any more.")
        }
        guard card.available(entry) else {
            return .result(dialog: "\(card.name) is asleep or out of reach.")
        }
        guard let run = await model.run(entry, on: card, params: params) else {
            return .result(dialog: "\(item.name) didn't run: \(model.toast?.text ?? "something went wrong").")
        }
        let output = run.output.trimmingCharacters(in: .whitespacesAndNewlines)
        if run.ok {
            return .result(dialog: output.isEmpty || output.count > 200 ? "Done." : "\(output)")
        }
        return .result(dialog: "\(item.name) failed\(run.exitCode.map { " with exit code \($0)" } ?? "").")
    }
}

struct WakeComputerIntent: AppIntent {
    static let title: LocalizedStringResource = "Wake a Computer"
    static let description = IntentDescription("Wakes a computer from sleep (on the same Wi-Fi), or turns its screen on.")

    @Parameter(title: "Computer")
    var computer: ComputerEntity

    @MainActor
    func perform() async throws -> some IntentResult & ProvidesDialog {
        let model = AppModel.shared
        await model.refresh()
        guard let card = model.cards.first(where: { $0.id == computer.id }) else {
            return .result(dialog: "I can't find \(computer.name).")
        }
        await model.wake(card)
        return .result(dialog: "\(model.toast?.text ?? "Done.")")
    }
}

struct CmdTriggerShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: RunItemIntent(),
            phrases: ["Run \(\.$item) with \(.applicationName)", "\(.applicationName) \(\.$item)"],
            shortTitle: "Run",
            systemImageName: "play.fill"
        )
        AppShortcut(
            intent: WakeComputerIntent(),
            phrases: ["Wake \(\.$computer) with \(.applicationName)", "Wake my computer with \(.applicationName)"],
            shortTitle: "Wake",
            systemImageName: "power"
        )
    }
}
