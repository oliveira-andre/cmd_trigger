import SwiftUI

/// DESIGN.md tokens: one near-black canvas, hairline-bordered cards (no fills, no shadows), a
/// single electric-green accent for primary actions and live status. SF Pro and SF Mono stand
/// in for Inter and SF Mono.
enum Theme {
    static let canvas = Color(hex: 0x101010)
    /// Inputs, code, sheets: just distinct from the canvas.
    static let canvasSoft = Color(hex: 0x1A1A1A)
    /// Cards are the canvas plus a hairline.
    static let card = canvas
    static let raised = canvasSoft
    static let hairline = Color(hex: 0x3D3A39)
    static let hairlineHover = Color(hex: 0x57534E)
    static let text = Color(hex: 0xF2F2F2)
    static let strong = Color.white
    static let codeText = Color(hex: 0xF5F6F7)
    static let muted = Color(hex: 0x8B949E)
    static let mutedStrong = Color(hex: 0xBDBDBD)
    static let primary = Color(hex: 0x00D992)
    static let primarySoft = Color(hex: 0x2FD6A1)
    static let primaryActive = Color(hex: 0x10B981)
    static let onPrimary = Color(hex: 0x101010)
    static let up = primary
    /// Only for saying something failed: status text and tinted tags, never a fill.
    static let down = Color(hex: 0xF6465D)
    static let pending = primarySoft

    static func number(_ size: CGFloat, _ weight: Font.Weight = .semibold) -> Font {
        .system(size: size, weight: weight, design: .monospaced)
    }
}

extension Color {
    init(hex: UInt32) {
        self.init(
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255
        )
    }
}

/// Near-black on green, the one main action. 6 px corners, 44 pt tall.
struct PrimaryButtonStyle: ButtonStyle {
    var pill = false
    var compact = false
    @Environment(\.isEnabled) private var enabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: compact ? 14 : 16, weight: .semibold))
            .padding(.horizontal, compact ? 12 : 16)
            .frame(height: compact ? 32 : 44)
            .foregroundStyle(enabled ? Theme.onPrimary : Theme.muted)
            .background(
                RoundedRectangle(cornerRadius: pill ? 22 : 6)
                    .fill(enabled ? (configuration.isPressed ? Theme.primaryActive : Theme.primary) : Theme.primary.opacity(0.25))
            )
    }
}

/// The hairline-on-dark secondary button.
struct SecondaryButtonStyle: ButtonStyle {
    var compact = false
    var destructive = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: compact ? 14 : 16, weight: .semibold))
            .padding(.horizontal, compact ? 12 : 16)
            .frame(height: compact ? 32 : 44)
            .foregroundStyle(destructive ? Theme.down : Theme.text)
            .background(RoundedRectangle(cornerRadius: 6).fill(configuration.isPressed ? Theme.canvasSoft : Theme.canvas))
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(Theme.hairline))
    }
}

/// A feature card: the canvas with a 1 px hairline, 8 px corners.
struct CardModifier: ViewModifier {
    var padding: CGFloat = 24
    func body(content: Content) -> some View {
        content
            .padding(padding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .hairline()
    }
}

extension View {
    func card(padding: CGFloat = 24) -> some View { modifier(CardModifier(padding: padding)) }

    /// The hairline rectangle every card-like surface uses. `edge` replaces the hairline (for
    /// "featured" or busy states).
    func hairline(radius: CGFloat = 8, fill: Color = Theme.canvas, edge: Color = Theme.hairline, width: CGFloat = 1) -> some View {
        background(RoundedRectangle(cornerRadius: radius).fill(fill))
            .overlay(RoundedRectangle(cornerRadius: radius).strokeBorder(edge, lineWidth: width))
    }
}
