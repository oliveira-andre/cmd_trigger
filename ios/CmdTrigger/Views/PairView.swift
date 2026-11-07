import SwiftUI
import VisionKit

/// Pairing with a computer: scan its QR code, or pick it on the Wi-Fi (or type its address)
/// and type the code it shows.
struct PairView: View {
    /// Preset by a `cmdtrigger://pair` link.
    var preset: PhoneLink.Target?
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var found: [PhoneLink.Target] = []
    @State private var searching = false
    @State private var target: PhoneLink.Target?
    @State private var address = ""
    @State private var code = ""
    @State private var working = false
    @State private var error: String?
    @State private var scanning = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    if let target {
                        chosen(target)
                    } else {
                        choose
                    }
                }
                .padding(20)
            }
            .background(Theme.canvasSoft)
            .navigationTitle("Pair a computer")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() }.tint(Theme.mutedStrong) }
            }
        }
        .formSheet()
        .task {
            if let preset {
                target = preset
                code = preset.code.map(PhoneCrypto.grouped) ?? ""
                if preset.code != nil { await submit() }
            } else {
                await search()
            }
        }
        .fullScreenCover(isPresented: $scanning) {
            QRScanner { url in
                scanning = false
                if let scanned = PhoneLink.Target(url: url) {
                    target = scanned
                    code = scanned.code.map(PhoneCrypto.grouped) ?? ""
                    Task { await submit() }
                }
            } cancel: {
                scanning = false
            }
        }
    }

    private var choose: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("On the computer, open CmdTrigger → Settings → iPhone → Pair an iPhone.")
                .font(.system(size: 15))
                .foregroundStyle(Theme.mutedStrong)
            if QRScanner.available {
                Button { scanning = true } label: {
                    Label("Scan the QR code", systemImage: "qrcode.viewfinder").frame(maxWidth: .infinity)
                }
                .buttonStyle(PrimaryButtonStyle())
                .accessibilityIdentifier("pair.scan")
            }
            HStack {
                SectionTitle(text: "On this Wi-Fi")
                if searching { ProgressView().controlSize(.small).tint(Theme.mutedStrong) }
                Button { Task { await search() } } label: { Image(systemName: "arrow.clockwise") }
                    .tint(Theme.mutedStrong)
                    .disabled(searching)
            }
            if found.isEmpty && !searching {
                Text("No computer found. Is CmdTrigger open on it, on this Wi-Fi?")
                    .font(.system(size: 14))
                    .foregroundStyle(Theme.muted)
            }
            ForEach(found, id: \.key) { computer in
                Button {
                    target = computer
                } label: {
                    HStack(spacing: 12) {
                        Image(systemName: "desktopcomputer").foregroundStyle(Theme.primary)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(computer.name).font(.system(size: 16, weight: .semibold)).foregroundStyle(Theme.strong)
                            Text(computer.hosts.first ?? "").font(.system(size: 12, design: .monospaced)).foregroundStyle(Theme.muted)
                        }
                        Spacer()
                        Image(systemName: "chevron.right").foregroundStyle(Theme.muted)
                    }
                    .padding(16)
                    .hairline(fill: Theme.canvas)
                }
                .accessibilityIdentifier("pair.found.\(computer.name)")
            }
            SectionTitle(text: "Or by address")
            HStack(spacing: 10) {
                FormField(placeholder: "192.168.1.20", text: $address, monospaced: true, keyboard: .numbersAndPunctuation)
                    .accessibilityIdentifier("pair.address")
                Button("Next") {
                    if let typed = PhoneLink.Target(address: address) { target = typed }
                }
                .buttonStyle(SecondaryButtonStyle())
                .disabled(PhoneLink.Target(address: address) == nil)
                .accessibilityIdentifier("pair.address.next")
            }
        }
    }

    private func chosen(_ target: PhoneLink.Target) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 12) {
                Image(systemName: "desktopcomputer")
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(Theme.primary)
                    .frame(width: 40, height: 40)
                    .hairline(fill: Theme.canvas)
                VStack(alignment: .leading, spacing: 2) {
                    Text(target.name).font(.system(size: 18, weight: .bold)).foregroundStyle(Theme.strong)
                    Text("\(target.hosts.first ?? ""):\(target.port)").font(.system(size: 12, design: .monospaced)).foregroundStyle(Theme.muted)
                }
                Spacer()
                if preset == nil {
                    Button("Change") {
                        self.target = nil
                        error = nil
                    }
                    .tint(Theme.mutedStrong)
                }
            }
            VStack(alignment: .leading, spacing: 6) {
                FieldLabel(text: "The code on the computer")
                FormField(placeholder: "XXXX-XXXX-XXXX-XXXX", text: $code, monospaced: true)
                    .textInputAutocapitalization(.characters)
                    .accessibilityIdentifier("pair.code")
                    .onSubmit { Task { await submit() } }
            }
            if let error {
                Label(error, systemImage: "exclamationmark.circle")
                    .font(.system(size: 14))
                    .foregroundStyle(Theme.down)
                    .accessibilityIdentifier("pair.error")
            }
            Button {
                Task { await submit() }
            } label: {
                HStack {
                    if working { ProgressView().tint(Theme.onPrimary) }
                    Text(working ? "Pairing…" : "Pair")
                }
                .frame(maxWidth: .infinity)
            }
            .buttonStyle(PrimaryButtonStyle())
            .disabled(working || PhoneCrypto.normalize(code) == nil)
            .accessibilityIdentifier("pair.submit")
            Text("The code works once, for a few minutes. Your iPhone and the computer then share a key; everything between them is encrypted.")
                .font(.system(size: 12))
                .foregroundStyle(Theme.muted)
        }
    }

    private func search() async {
        searching = true
        found = await PhoneLink.discover(seconds: 2.5)
        searching = false
    }

    private func submit() async {
        guard let target, !working else { return }
        working = true
        error = nil
        do {
            try await model.pair(target, code: code)
            Haptics.success()
            dismiss()
        } catch {
            self.error = error.localizedDescription
            Haptics.failure()
        }
        working = false
    }
}

/// The camera, looking for a CmdTrigger pairing QR code.
struct QRScanner: View {
    var found: (URL) -> Void
    var cancel: () -> Void

    @MainActor static var available: Bool { DataScannerViewController.isSupported && DataScannerViewController.isAvailable }

    var body: some View {
        ZStack(alignment: .top) {
            Representable(found: found).ignoresSafeArea()
            HStack {
                Text("Point at the QR code on the computer").font(.system(size: 15, weight: .semibold)).foregroundStyle(.white)
                Spacer()
                Button("Cancel", action: cancel).tint(.white)
            }
            .padding(16)
            .background(.black.opacity(0.55))
        }
    }

    private struct Representable: UIViewControllerRepresentable {
        var found: (URL) -> Void

        func makeUIViewController(context: Context) -> DataScannerViewController {
            let scanner = DataScannerViewController(recognizedDataTypes: [.barcode(symbologies: [.qr])], isHighlightingEnabled: true)
            scanner.delegate = context.coordinator
            try? scanner.startScanning()
            return scanner
        }

        func updateUIViewController(_ controller: DataScannerViewController, context: Context) {}

        func makeCoordinator() -> Coordinator { Coordinator(found: found) }

        final class Coordinator: NSObject, DataScannerViewControllerDelegate {
            let found: (URL) -> Void
            private var done = false
            init(found: @escaping (URL) -> Void) { self.found = found }

            func dataScanner(_ scanner: DataScannerViewController, didAdd items: [RecognizedItem], allItems: [RecognizedItem]) {
                for item in items {
                    if case let .barcode(code) = item, let text = code.payloadStringValue,
                       let url = URL(string: text), url.scheme == "cmdtrigger", !done {
                        done = true
                        scanner.stopScanning()
                        found(url)
                    }
                }
            }
        }
    }
}
