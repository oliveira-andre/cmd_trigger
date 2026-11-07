import SwiftUI

struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @State private var pairing = false
    @State private var url = ""
    @State private var email = ""
    @State private var password = ""
    @State private var signingIn = false
    @State private var error: String?
    @State private var removing: PairedComputer?

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 22) {
                    Text("Settings")
                        .font(.system(size: 24, weight: .bold))
                        .tracking(-0.6)
                        .foregroundStyle(Theme.strong)
                        .padding(.top, 8)
                    computers
                    server
                    siri
                    Text("CmdTrigger \(Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "")")
                        .font(.system(size: 12))
                        .foregroundStyle(Theme.muted)
                        .frame(maxWidth: .infinity)
                }
                .padding(.horizontal, 16)
                .padding(.bottom, 24)
            }
            .background(Theme.canvas)
            .toolbar(.hidden, for: .navigationBar)
        }
        .sheet(isPresented: $pairing) { PairView() }
        .confirmationDialog("Remove \(removing?.name ?? "")?", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }), titleVisibility: .visible) {
            Button("Remove", role: .destructive) {
                if let computer = removing { Task { await model.unpair(computer) } }
                removing = nil
            }
        } message: {
            Text("This iPhone forgets it, and it forgets this iPhone. You can pair again any time.")
        }
    }

    private var computers: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Computers on your Wi-Fi")
            ForEach(model.paired) { computer in
                HStack(spacing: 12) {
                    Image(systemName: "desktopcomputer").foregroundStyle(Theme.primary)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(computer.name).font(.system(size: 16, weight: .semibold)).foregroundStyle(Theme.strong)
                        Text(detail(computer)).font(.system(size: 12)).foregroundStyle(Theme.muted)
                    }
                    Spacer()
                    Button("Remove") { removing = computer }
                        .font(.system(size: 13, weight: .medium))
                        .tint(Theme.down)
                }
                .padding(14)
                .hairline()
            }
            Button { pairing = true } label: { Label("Pair a computer", systemImage: "plus").frame(maxWidth: .infinity) }
                .buttonStyle(SecondaryButtonStyle())
                .accessibilityIdentifier("settings.pair")
        }
    }

    private func detail(_ computer: PairedComputer) -> String {
        var parts = [computer.hosts.first.map { "\($0):\(computer.port)" } ?? ""]
        switch computer.wakeForNetwork {
        case true?: parts.append("wakes over Wi-Fi")
        case false?: parts.append("Wake for network access is off")
        case nil: break
        }
        if computer.server != nil { parts.append("on your server") }
        return parts.filter { !$0.isEmpty }.joined(separator: " · ")
    }

    @ViewBuilder private var server: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Your server (away from home)")
            if let account = model.account {
                VStack(alignment: .leading, spacing: 10) {
                    HStack(spacing: 10) {
                        Image(systemName: model.serverError == nil ? "checkmark.icloud" : "icloud.slash")
                            .foregroundStyle(model.serverError == nil ? Theme.up : Theme.down)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(account.url).font(.system(size: 14, design: .monospaced)).foregroundStyle(Theme.strong).lineLimit(1)
                            Text(model.serverError ?? "\(account.email) · \(model.serverComputers.count) computer(s)")
                                .font(.system(size: 12))
                                .foregroundStyle(Theme.muted)
                        }
                    }
                    Button("Sign out") { model.signOut() }
                        .buttonStyle(SecondaryButtonStyle(compact: true, destructive: true))
                }
                .card(padding: 14)
            } else {
                VStack(alignment: .leading, spacing: 10) {
                    Text("Sign in to your CmdTrigger server to run things when you're not home. The iPhone keeps a token, not your password.")
                        .font(.system(size: 13))
                        .foregroundStyle(Theme.mutedStrong)
                    FormField(placeholder: "cmd.example.com", text: $url, monospaced: true, keyboard: .URL)
                        .accessibilityIdentifier("server.url")
                    FormField(placeholder: "you@example.com", text: $email, keyboard: .emailAddress)
                        .accessibilityIdentifier("server.email")
                    FormField(placeholder: "Password", text: $password, secure: true)
                        .accessibilityIdentifier("server.password")
                    if let error {
                        Text(error).font(.system(size: 13)).foregroundStyle(Theme.down)
                    }
                    Button {
                        Task { await signIn() }
                    } label: {
                        HStack {
                            if signingIn { ProgressView().tint(Theme.onPrimary) }
                            Text("Sign in")
                        }.frame(maxWidth: .infinity)
                    }
                    .buttonStyle(PrimaryButtonStyle())
                    .disabled(signingIn || url.isEmpty || email.isEmpty || password.isEmpty)
                    .accessibilityIdentifier("server.signin")
                }
                .card(padding: 14)
            }
        }
    }

    private var siri: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionTitle(text: "Siri & Shortcuts")
            VStack(alignment: .leading, spacing: 8) {
                Text("“Hey Siri, CmdTrigger Morning routine”")
                Text("“Hey Siri, wake my computer with CmdTrigger”")
                Text("In Shortcuts, find CmdTrigger’s Run and Wake actions to build your own routines, or run them from an automation (an alarm, arriving home, a Focus).")
                    .foregroundStyle(Theme.muted)
            }
            .font(.system(size: 14))
            .foregroundStyle(Theme.mutedStrong)
            .card(padding: 14)
        }
    }

    private func signIn() async {
        signingIn = true
        error = nil
        do {
            try await model.signIn(url: url, email: email, password: password)
            password = ""
            await model.refresh()
        } catch {
            self.error = error.localizedDescription
        }
        signingIn = false
    }
}
