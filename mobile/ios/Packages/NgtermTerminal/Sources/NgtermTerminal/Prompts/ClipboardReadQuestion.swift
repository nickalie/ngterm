import NgtermDesign
import SwiftUI

/// The question a program's clipboard read asks (mobile/android's `ClipboardReadQuestion`), a sheet's page: the
/// attention glyph, "Let docs-site read your clipboard?" as a heading, what the program would see (the text, or only
/// what kinds it holds when `listOnly`) and from where, then **Allow** (prominent), and **Always allow on build-01**
/// and **Deny**, quieter (the tint on the ground).
public struct ClipboardReadQuestion: View {
    private static let width: Double = 520

    private let listOnly: Bool
    private let machine: String
    private let session: String
    private let onAllow: @MainActor () -> Void
    private let onAlways: @MainActor () -> Void
    private let onDeny: @MainActor () -> Void

    /// The page.
    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.large) {
                Image(systemName: StatusTone.attention.shape.symbolName)
                    .font(.title2)
                    .foregroundStyle(StatusTone.attention.color.color)
                    .accessibilityHidden(true)
                Text("Let \(session) read your clipboard?")
                    .font(.title2.bold())
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.isHeader)
                Text(explanation)
                    .foregroundStyle(Palette.secondaryText.color)
                    .fixedSize(horizontal: false, vertical: true)
                answers
                    .padding(.top, Spacing.small)
            }
            .frame(maxWidth: Self.width, alignment: .leading)
            .padding(.horizontal, Spacing.xLarge)
            .padding(.vertical, Spacing.large)
            .frame(maxWidth: .infinity)
        }
        .background(Color(uiColor: .systemGroupedBackground))
    }

    private var explanation: String {
        if listOnly {
            String(
                localized: """
                    A program in \(session) on \(machine) asks what kind of content your clipboard holds. It doesn't \
                    see the text itself.
                    """
            )
        } else {
            String(
                localized: """
                    A program in \(session) on \(machine) asks for the text you copied. It sees it only if you allow \
                    it.
                    """
            )
        }
    }

    private var answers: some View {
        VStack(spacing: Spacing.small) {
            Button {
                onAllow()
            } label: {
                Text("Allow")
                    .frame(maxWidth: .infinity)
            }
            .prominentAction()
            .accessibilityIdentifier("clipboard-allow")
            quiet("Always allow on \(machine)", identifier: "clipboard-always", action: onAlways)
            quiet("Deny", identifier: "clipboard-deny", action: onDeny)
        }
    }

    /// The question of a read from `session` on `machine` (by their names), asking only what kinds the clipboard holds
    /// when `listOnly`; each answer runs its closure.
    @preconcurrency
    public init(
        listOnly: Bool,
        machine: String,
        session: String,
        onAllow: @escaping @MainActor () -> Void,
        onAlways: @escaping @MainActor () -> Void,
        onDeny: @escaping @MainActor () -> Void
    ) {
        self.listOnly = listOnly
        self.machine = machine
        self.session = session
        self.onAllow = onAllow
        self.onAlways = onAlways
        self.onDeny = onDeny
    }

    // a quieter answer: the tint on the ground, a full-width 44 pt target (a tinted fill under it stays under 4.5:1)
    private func quiet(
        _ title: LocalizedStringKey,
        identifier: String,
        action: @escaping @MainActor () -> Void
    ) -> some View {
        Button {
            action()
        } label: {
            Text(title)
                .frame(maxWidth: .infinity, minHeight: Spacing.hitTarget)
                .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .accessibilityIdentifier(identifier)
    }
}
