import SwiftUI

struct HomeView: View {
    @Environment(AppModel.self) private var model
    @State private var asking: Asking?
    @State private var pairing = false

    struct Asking: Identifiable {
        var card: ComputerCard
        var entry: CatalogEntry
        var id: String { "\(card.id)|\(entry.key)" }
    }

    private let columns = [GridItem(.flexible(), spacing: 10), GridItem(.flexible(), spacing: 10)]

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 18) {
                    header
                    if model.isEmpty {
                        welcome
                    } else {
                        ForEach(model.cards) { card in computer(card) }
                        if let error = model.serverError {
                            Label(error, systemImage: "icloud.slash")
                                .font(.system(size: 13))
                                .foregroundStyle(Theme.muted)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.bottom, 24)
            }
            .scrollIndicators(.hidden)
            .background(Theme.canvas)
            .refreshable { await model.refresh() }
            .toolbar(.hidden, for: .navigationBar)
        }
        .sheet(item: $asking) { asking in
            AskSheet(card: asking.card, entry: asking.entry)
        }
        .sheet(isPresented: $pairing) { PairView() }
    }

    private var header: some View {
        HStack(spacing: 10) {
            Image("Mark").resizable().frame(width: 28, height: 28)
            Text("CmdTrigger").font(.system(size: 24, weight: .bold)).tracking(-0.6).foregroundStyle(Theme.strong)
            Spacer()
            if model.refreshing { ProgressView().tint(Theme.mutedStrong) }
            Button {
                pairing = true
            } label: {
                Image(systemName: "plus")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(Theme.strong)
                    .frame(width: 36, height: 36)
                    .background(Circle().fill(Theme.canvas))
                    .overlay(Circle().strokeBorder(Theme.hairline))
            }
            .accessibilityIdentifier("home.add")
        }
        .padding(.top, 8)
    }

    private var welcome: some View {
        VStack(alignment: .leading, spacing: 14) {
            Image(systemName: "desktopcomputer.and.arrow.down")
                .font(.system(size: 28, weight: .semibold))
                .foregroundStyle(Theme.primary)
            Text("Pair your computer").font(.system(size: 20, weight: .bold)).foregroundStyle(Theme.strong)
            Text("On your Mac, open CmdTrigger → Settings → iPhone → Pair an iPhone. Scan the QR code with this iPhone's camera, or tap below and type the code.")
                .font(.system(size: 15))
                .foregroundStyle(Theme.mutedStrong)
            Button("Pair a computer") { pairing = true }
                .buttonStyle(PrimaryButtonStyle())
                .accessibilityIdentifier("home.pair")
            Text("Away from home? Sign in to your CmdTrigger server in Settings.")
                .font(.system(size: 13))
                .foregroundStyle(Theme.muted)
        }
        .card()
    }

    private func computer(_ card: ComputerCard) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 12) {
                Image(systemName: card.symbol)
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(Theme.strong)
                    .frame(width: 40, height: 40)
                    .hairline(fill: Theme.canvasSoft)
                VStack(alignment: .leading, spacing: 4) {
                    Text(card.name).font(.system(size: 18, weight: .bold)).foregroundStyle(Theme.strong).lineLimit(1)
                    ReachChip(reach: card.reach)
                }
                Spacer()
                if card.paired != nil { wakeButton(card) }
            }
            if card.catalog.isEmpty {
                Text(card.reach == .checking ? "Loading…" : "Nothing to run yet. Add commands and automations in CmdTrigger on the computer.")
                    .font(.system(size: 14))
                    .foregroundStyle(Theme.muted)
            }
            if !card.routines.isEmpty {
                SectionTitle(text: "Routines")
                grid(card, card.routines)
            }
            if !card.commands.isEmpty {
                SectionTitle(text: "Commands")
                grid(card, card.commands)
            }
        }
        .card(padding: 16)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("computer.\(card.name)")
    }

    private func wakeButton(_ card: ComputerCard) -> some View {
        let asleep = card.reach == .asleep || card.reach == .server
        return Button {
            Haptics.tap()
            Task { await model.wake(card) }
        } label: {
            HStack(spacing: 6) {
                if model.isWaking(card) {
                    ProgressView().controlSize(.small).tint(asleep ? Theme.onPrimary : Theme.strong)
                } else {
                    Image(systemName: asleep ? "power" : "sun.max")
                }
                Text(model.isWaking(card) ? "Waking…" : asleep ? "Wake" : "Screen on")
            }
        }
        .buttonStyle(asleep ? AnyButtonStyle(PrimaryButtonStyle(compact: true)) : AnyButtonStyle(SecondaryButtonStyle(compact: true)))
        .disabled(model.isWaking(card) || card.reach == .checking)
        .accessibilityIdentifier("wake.\(card.name)")
    }

    private func grid(_ card: ComputerCard, _ entries: [CatalogEntry]) -> some View {
        LazyVGrid(columns: columns, spacing: 10) {
            ForEach(entries) { entry in
                EntryTile(entry: entry, available: card.available(entry), busy: model.isBusy(card, entry)) {
                    Haptics.tap()
                    if entry.asks {
                        asking = Asking(card: card, entry: entry)
                    } else {
                        Task { await model.run(entry, on: card) }
                    }
                }
            }
        }
    }
}

/// Wraps either button style so the wake button can switch between them.
struct AnyButtonStyle: ButtonStyle {
    private let make: (Configuration) -> AnyView
    init<S: ButtonStyle>(_ style: S) { make = { AnyView(style.makeBody(configuration: $0)) } }
    func makeBody(configuration: Configuration) -> some View { make(configuration) }
}

/// Parameters, or on/off, before running.
struct AskSheet: View {
    var card: ComputerCard
    var entry: CatalogEntry
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var params = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 10) {
                if !entry.icon.isEmpty { Text(entry.icon).font(.system(size: 26)) }
                VStack(alignment: .leading, spacing: 2) {
                    Text(entry.name).font(.system(size: 20, weight: .bold)).foregroundStyle(Theme.strong)
                    Text("\(entry.isRoutine ? "Routine" : "Command") on \(card.name)").font(.system(size: 13)).foregroundStyle(Theme.muted)
                }
            }
            if !entry.description.isEmpty {
                Text(entry.description).font(.system(size: 15)).foregroundStyle(Theme.mutedStrong)
            }
            if entry.hasOff {
                HStack(spacing: 10) {
                    Button { go("off") } label: { Label("Off", systemImage: "power").frame(maxWidth: .infinity) }
                        .buttonStyle(SecondaryButtonStyle())
                        .accessibilityIdentifier("ask.off")
                    Button { go("on") } label: { Label("On", systemImage: "power").frame(maxWidth: .infinity) }
                        .buttonStyle(PrimaryButtonStyle())
                        .accessibilityIdentifier("ask.on")
                }
            }
            if entry.allowParams {
                VStack(alignment: .leading, spacing: 6) {
                    FieldLabel(text: entry.hasOff ? "Or a value" : "Parameters")
                    FormField(placeholder: "Parameters", text: $params)
                        .accessibilityIdentifier("ask.params")
                        .onSubmit { go(params) }
                }
                Button { go(params) } label: { Label("Run", systemImage: "play.fill").frame(maxWidth: .infinity) }
                    .buttonStyle(PrimaryButtonStyle())
                    .accessibilityIdentifier("ask.run")
            }
        }
        .padding(20)
        .presentationDetents([.height(entry.allowParams && entry.hasOff ? 330 : 270)])
        .formSheet()
    }

    private func go(_ value: String?) {
        dismiss()
        Task { await model.run(entry, on: card, params: value) }
    }
}
