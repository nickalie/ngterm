import NgtermCore
import NgtermDesign
import SwiftUI
import UIKit

/// The question a program's clipboard read asks (mobile/android's `ClipboardReadQuestion`), a sheet's page: the
/// attention glyph, "Let docs-site read your clipboard?" as a heading, from where, what the program would see (the
/// text or what kinds of content and how much, and the first image, on a plate: Ghostty's macOS form), the "don't ask
/// again" toggle when the read can be remembered, then **Allow** (prominent), and **Always allow on build-01** and
/// **Deny**, quieter (the tint on the ground).
public struct ClipboardReadQuestion: View {
    private static let width: Double = 520
    // the tallest the copied text shows before it scrolls, and an image (Ghostty's dialog: 200 points)
    private static let textHeight: Double = 160
    private static let imageHeight: Double = 200

    private let canRemember: Bool
    private let machine: String
    private let session: String
    private let preview: [ClipboardPart]
    private let listed: Bool
    private let onAllow: @MainActor (Bool) -> Void
    private let onAlways: @MainActor (Bool) -> Void
    private let onDeny: @MainActor () -> Void
    @State private var remember = false

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
                plate
                if canRemember {
                    Toggle("Don't ask again for this program in this terminal", isOn: $remember)
                        .accessibilityIdentifier("clipboard-remember")
                }
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
        String(
            localized: """
                A program in \(session) on \(machine) asks for what you copied. It sees this only if you allow it.
                """
        )
    }

    // what the read would hand over, on the grouped ground's raised surface
    private var plate: some View {
        VStack(alignment: .leading, spacing: Spacing.small) {
            plateContent
        }
        .padding(Spacing.medium)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            Color(uiColor: .secondarySystemGroupedBackground),
            in: RoundedRectangle(cornerRadius: Radius.medium, style: .continuous)
        )
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("clipboard-preview")
    }

    @ViewBuilder private var plateContent: some View {
        let summary = ClipboardSummary(preview)
        if let text = summary.text {
            ScrollView {
                Text(verbatim: text)
                    .font(Typography.plateValue)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: Self.textHeight)
            .fixedSize(horizontal: false, vertical: true)
        } else {
            Text(noneHeld)
                .foregroundStyle(Palette.secondaryText.color)
                .fixedSize(horizontal: false, vertical: true)
        }
        if let part = summary.image, let image = UIImage(data: part.data) {
            Image(uiImage: image)
                .resizable()
                .interpolation(.none)
                .scaledToFit()
                .frame(maxHeight: Self.imageHeight, alignment: .leading)
                .accessibilityLabel(Text("Copied image, \(part.mime)"))
        }
    }

    private var noneHeld: String {
        if listed {
            String(
                localized: """
                    Your clipboard holds none of the kinds of content the program asks for. It sees only which kinds \
                    your clipboard holds.
                    """
            )
        } else {
            String(localized: "Your clipboard holds none of the kinds of content the program asks for.")
        }
    }

    private var answers: some View {
        VStack(spacing: Spacing.small) {
            Button {
                onAllow(remember)
            } label: {
                Text("Allow")
                    .frame(maxWidth: .infinity)
            }
            .prominentAction()
            .accessibilityIdentifier("clipboard-allow")
            quiet("Always allow on \(machine)", identifier: "clipboard-always") {
                onAlways(remember)
            }
            quiet("Deny", identifier: "clipboard-deny", action: onDeny)
        }
    }

    /// The question of a read from `session` on `machine` (by their names) that would hand over `preview` (`listed`:
    /// the read also learns which types the clipboard holds), offering the "don't ask again" toggle when it
    /// `canRemember`; each answer runs its closure, Allow and Always with the toggle's state.
    @preconcurrency
    public init(
        canRemember: Bool,
        machine: String,
        session: String,
        preview: [ClipboardPart],
        listed: Bool,
        onAllow: @escaping @MainActor (Bool) -> Void,
        onAlways: @escaping @MainActor (Bool) -> Void,
        onDeny: @escaping @MainActor () -> Void
    ) {
        self.canRemember = canRemember
        self.machine = machine
        self.session = session
        self.preview = preview
        self.listed = listed
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
