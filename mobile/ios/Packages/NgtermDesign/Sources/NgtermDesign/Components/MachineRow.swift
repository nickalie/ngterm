import SwiftUI

/// A machine in a list: its name, its connection's badge, its mono address (with the app it runs after it, when that
/// isn't ngterm: a ``PeerBadge``), what the state means with the one action it calls for, and a slot for its sessions'
/// statuses while it is connected.
///
/// It shows what it is given and reads no app state; the list it sits in makes it a row, a card in an inset list. At an
/// accessibility text size the badge goes under the name. VoiceOver reads it as one element, its name a heading, with
/// the action as the element's custom action.
public struct MachineRow<Summary: View>: View {
    @Environment(\.dynamicTypeSize)
    private var typeSize

    private let name: String
    private let address: String
    private let peer: String?
    private let tone: StatusTone
    private let toneLabel: String?
    private let detail: String?
    private let action: RowAction?
    private let summary: Summary

    /// The body of the view.
    public var body: some View {
        VStack(alignment: .leading, spacing: Spacing.xSmall) {
            let heading =
                typeSize.isAccessibilitySize
                ? AnyLayout(VStackLayout(alignment: .leading, spacing: Spacing.xSmall))
                : AnyLayout(HStackLayout(alignment: .firstTextBaseline, spacing: Spacing.small))
            heading {
                Text(name)
                    .font(Typography.machineName)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.isHeader)
                if !typeSize.isAccessibilitySize {
                    Spacer(minLength: 0)
                }
                StatusBadge(tone, label: toneLabel)
            }
            addressLine
            if detail != nil || action != nil {
                DetailLine(detail: detail, action: action)
            }
            summary
        }
        .padding(.vertical, Spacing.xSmall)
        .frame(minHeight: Spacing.hitTarget)
        // one element, its action a custom action of it (the line's button is hidden from VoiceOver)
        .accessibilityElement(children: .combine)
        .accessibilityActions {
            if let action {
                Button(action.title, action: action.perform)
            }
        }
    }

    // the badge goes under the address at an accessibility size rather than cutting it short
    @ViewBuilder private var addressLine: some View {
        let line =
            typeSize.isAccessibilitySize
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: Spacing.xSmall))
            : AnyLayout(HStackLayout(alignment: .center, spacing: Spacing.small))
        line {
            Text(verbatim: address)
                .font(Typography.address)
                .rowForeground(Palette.secondaryText.color)
            if let peer {
                PeerBadge(peer)
            }
        }
    }

    /// A machine named `name` at `address` (`user@host:port`), its connection `tone` (with `toneLabel` in place of
    /// its word), what the state means (`detail`) and its `action`, the app it runs (`peer`, an app's name; `nil` for
    /// ngterm), then `summary`.
    public init(
        name: String,
        address: String,
        tone: StatusTone,
        toneLabel: String? = nil,
        detail: String? = nil,
        action: RowAction? = nil,
        peer: String? = nil,
        @ViewBuilder summary: () -> Summary
    ) {
        self.name = name
        self.address = address
        self.peer = peer
        self.tone = tone
        self.toneLabel = toneLabel
        self.detail = detail
        self.action = action
        self.summary = summary()
    }
}

extension MachineRow where Summary == EmptyView {
    /// A machine with no sessions' summary: not connected, or none to show.
    public init(
        name: String,
        address: String,
        tone: StatusTone,
        toneLabel: String? = nil,
        detail: String? = nil,
        action: RowAction? = nil,
        peer: String? = nil
    ) {
        self.init(
            name: name,
            address: address,
            tone: tone,
            toneLabel: toneLabel,
            detail: detail,
            action: action,
            peer: peer
        ) {
            EmptyView()
        }
    }
}
