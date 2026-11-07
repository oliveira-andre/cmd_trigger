import SwiftUI

enum Format {
    static func ago(_ date: Date) -> String {
        let delta = max(0, Date().timeIntervalSince(date))
        if delta < 45 { return "just now" }
        if delta < 3600 { return "\(Int((delta / 60).rounded())) min ago" }
        if delta < 86400 { return "\(Int((delta / 3600).rounded())) h ago" }
        return date.formatted(date: .abbreviated, time: .shortened)
    }
}

/// The section labels on the home screen ("ROUTINES", "COMMANDS").
struct SectionTitle: View {
    var text: String
    var body: some View {
        Text(text.uppercased())
            .font(.system(size: 12, weight: .semibold))
            .tracking(2.16)
            .foregroundStyle(Theme.muted)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// How a computer can be reached, as a coloured dot and words.
struct ReachChip: View {
    var reach: Reach
    var body: some View {
        HStack(spacing: 6) {
            if reach == .checking {
                ProgressView().controlSize(.mini).tint(Theme.mutedStrong)
            } else if reach == .server {
                // Reachable, but not directly: a ring rather than a dot.
                Circle().strokeBorder(Theme.primary, lineWidth: 1.5).frame(width: 8, height: 8)
            } else {
                Circle().fill(color).frame(width: 7, height: 7)
            }
            Text(reach.label).font(.system(size: 12, weight: .medium)).foregroundStyle(Theme.mutedStrong)
        }
        .padding(.horizontal, 10)
        .frame(height: 24)
        .background(Capsule().fill(Theme.canvas))
        .overlay(Capsule().strokeBorder(Theme.hairline))
    }

    private var color: Color {
        switch reach {
        case .home: Theme.up
        case .server: Theme.primary
        case .asleep, .checking: Theme.muted
        }
    }
}

struct StatusChip: View {
    var status: RunItem.Status
    var body: some View {
        Text(status.rawValue.replacingOccurrences(of: "_", with: " "))
            .font(.system(size: 11, weight: .semibold))
            .padding(.horizontal, 9)
            .frame(height: 22)
            .foregroundStyle(color)
            .background(Capsule().fill(color.opacity(0.1)))
            .overlay(Capsule().strokeBorder(color.opacity(0.35)))
    }

    private var color: Color {
        switch status {
        case .succeeded: Theme.up
        case .queued, .running: Theme.pending
        default: Theme.down
        }
    }
}

/// One command or routine: tap to run.
struct EntryTile: View {
    var entry: CatalogEntry
    var available: Bool
    var busy: Bool
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    glyph
                    Spacer()
                    if busy {
                        ProgressView().controlSize(.small).tint(Theme.primary)
                    } else if entry.asks {
                        Image(systemName: entry.hasOff ? "power" : "slider.horizontal.3")
                            .font(.system(size: 12, weight: .semibold))
                            .foregroundStyle(Theme.muted)
                    }
                }
                Spacer(minLength: 0)
                Text(entry.name)
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(available ? Theme.strong : Theme.muted)
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(16)
            .frame(height: 104)
            .hairline(edge: busy ? Theme.primary : Theme.hairline, width: busy ? 2 : 1)
            .opacity(available ? 1 : 0.5)
        }
        .buttonStyle(PressStyle())
        .disabled(!available || busy)
        .accessibilityIdentifier("tile.\(entry.name)")
    }

    @ViewBuilder private var glyph: some View {
        if !entry.icon.isEmpty {
            Text(entry.icon).font(.system(size: 24))
        } else {
            Image(systemName: entry.isRoutine ? "sparkles" : "terminal")
                .font(.system(size: 18, weight: .semibold))
                .foregroundStyle(Theme.primary)
                .frame(width: 30, height: 30)
                .hairline(radius: 6, fill: Theme.canvasSoft)
        }
    }
}

/// Shrinks a little while pressed.
struct PressStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.97 : 1)
            .animation(.easeOut(duration: 0.12), value: configuration.isPressed)
    }
}

struct ToastView: View {
    var toast: AppModel.Toast
    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: toast.ok ? "checkmark.circle.fill" : "exclamationmark.circle.fill")
                .foregroundStyle(toast.ok ? Theme.up : Theme.down)
            Text(toast.text)
                .font(.system(size: 14, weight: .medium))
                .foregroundStyle(Theme.strong)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .hairline(fill: Theme.canvasSoft, edge: (toast.ok ? Theme.up : Theme.down).opacity(0.45))
        .shadow(color: .black.opacity(0.5), radius: 12, y: 6)
        .padding(.horizontal, 16)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("toast")
    }
}

/// Sheets sit on the dark card surface, like the rest of the app.
struct FormSheet: ViewModifier {
    func body(content: Content) -> some View {
        content
            .presentationBackground(Theme.canvasSoft)
            .presentationDragIndicator(.visible)
    }
}

extension View {
    func formSheet() -> some View { modifier(FormSheet()) }
}

struct FieldLabel: View {
    var text: String
    var body: some View {
        Text(text).font(.system(size: 12, weight: .medium)).foregroundStyle(Theme.muted)
    }
}

struct FormField: View {
    var placeholder: String
    @Binding var text: String
    var monospaced = false
    var secure = false
    var keyboard: UIKeyboardType = .default

    var body: some View {
        Group {
            if secure {
                SecureField(placeholder, text: $text)
            } else {
                TextField(placeholder, text: $text)
                    .keyboardType(keyboard)
            }
        }
        .textInputAutocapitalization(.never)
        .autocorrectionDisabled()
        .font(monospaced ? .system(size: 16, design: .monospaced) : .system(size: 16))
        .foregroundStyle(Theme.strong)
        .padding(.horizontal, 16)
        .frame(height: 44)
        .hairline(radius: 6, fill: Theme.canvas)
    }
}

/// A finished run: status, exit code, output.
struct RunDetail: View {
    var run: RunItem
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .firstTextBaseline) {
                Text(run.name).font(.system(size: 20, weight: .bold)).foregroundStyle(Theme.strong)
                Spacer()
                StatusChip(status: run.status)
            }
            Text([run.computer, run.params.map { "“\($0)”" }, run.exitCode.map { "exit \($0)" }, run.source, Format.ago(run.at)]
                .compactMap { $0 }.joined(separator: " · "))
                .font(.system(size: 13))
                .foregroundStyle(Theme.muted)
            ScrollView {
                Text(run.output.isEmpty ? "(no output)" : run.output)
                    .font(.system(size: 13, design: .monospaced))
                    .foregroundStyle(Theme.codeText)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(16)
            }
            .hairline(fill: Theme.canvas)
            .accessibilityIdentifier("run.output")
            Button("Close") { dismiss() }
                .buttonStyle(SecondaryButtonStyle())
                .frame(maxWidth: .infinity, alignment: .trailing)
        }
        .padding(20)
        .presentationDetents([.medium, .large])
        .formSheet()
    }
}

enum Haptics {
    static func tap() { UIImpactFeedbackGenerator(style: .light).impactOccurred() }
    static func success() { UINotificationFeedbackGenerator().notificationOccurred(.success) }
    static func failure() { UINotificationFeedbackGenerator().notificationOccurred(.error) }
}
