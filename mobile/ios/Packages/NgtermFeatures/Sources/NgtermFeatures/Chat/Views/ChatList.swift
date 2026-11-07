import NgtermDesign
import SwiftUI

/// A chat's rows, oldest first, in one column no wider than a reading width (mobile/android's `ChatList`).
///
/// The header on top asks for the page before (`onLoadOlder`) as soon as it shows, then each row. The list opens at the
/// newest row and follows new ones (and a new height, the keyboard's) while it is at the end; a finger on it lets it be
/// at once, and a scroll that stops at the end follows again. Asked to (``ChatSnapshot/toEnd``: a notification's tap)
/// it goes back there and follows again. A page that goes in above keeps the row that was on top in place
/// (``ChatSnapshot/anchor``) while the list is still at its top. Which rows are open survives scrolling and the size
/// class, within their conversation.
internal struct ChatList: View {
    // the header's id in the list, and the list's end's
    private static let olderId = "older"
    private static let endId = "end"
    // how near the end a scroll that stops still follows
    private static let endSlack: CGFloat = 24

    internal let snapshot: ChatSnapshot
    internal let onLoadOlder: @MainActor () -> Void
    internal let onEndShown: @MainActor () -> Void
    @Binding internal var open: OpenRows
    @State private var isFollowing = true
    @State private var isAtEnd = true
    @State private var isAtTop = false

    internal var body: some View {
        GeometryReader { geometry in
            let column = min(Spacing.readableWidth, max(geometry.size.width - Spacing.large - Spacing.large, 0))
            list(column: column, width: geometry.size.width)
                .frame(width: geometry.size.width, height: geometry.size.height)
        }
        .clipped()
    }

    // the rows in one column `column` wide, centered in the list `width` wide: the scroll view lays its content out at
    // the content's own width, so both are given
    private func list(column: CGFloat, width: CGFloat) -> some View {
        let conversation = snapshot.conversation?.id
        let expanded = open.of(conversation)
        return ScrollViewReader { proxy in
            scroller(column: column, width: width, expanded: expanded, conversation: conversation, proxy: proxy)
        }
    }

    // the scroll view, which `proxy` moves: a scroll position bound to it shifts its content sideways
    private func scroller(
        column: CGFloat,
        width: CGFloat,
        expanded: Set<String>,
        conversation: String?,
        proxy: ScrollViewProxy
    ) -> some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                OlderHeader(older: snapshot.older, cwd: snapshot.conversation?.cwd, onRetry: onLoadOlder)
                    .id(Self.olderId)
                    .onAppear {
                        if snapshot.older == .more, !snapshot.log.isEmpty {
                            onLoadOlder()
                        }
                    }
                ForEach(snapshot.items) { item in
                    ChatRow(item: item, expanded: expanded) { key in
                        open = open.toggled(conversation, key)
                    }
                    .padding(.vertical, gap(item))
                    .id(item.key)
                }
                Color.clear
                    .frame(height: 1)
                    .id(Self.endId)
                    .accessibilityHidden(true)
            }
            .scrollTargetLayout()
            .frame(width: column)
            .padding(.vertical, Spacing.small)
            .frame(width: width)
        }
        .defaultScrollAnchor(.bottom)
        .onScrollGeometryChange(for: Bool.self) { geometry in
            geometry.contentOffset.y + geometry.containerSize.height
                >= geometry.contentSize.height + geometry.contentInsets.bottom - Self.endSlack
        } action: { _, atEnd in
            isAtEnd = atEnd
        }
        .onScrollGeometryChange(for: Bool.self) { geometry in
            geometry.contentOffset.y + geometry.contentInsets.top <= Self.endSlack
        } action: { _, atTop in
            isAtTop = atTop
        }
        .onScrollGeometryChange(for: CGFloat.self) { geometry in
            geometry.containerSize.height
        } action: { _, _ in
            // the list grew shorter (the keyboard came up) or taller: one that follows stays at its end
            if isFollowing {
                proxy.scrollTo(Self.endId, anchor: .bottom)
            }
        }
        .onScrollPhaseChange { _, phase in
            switch phase {
            case .interacting:
                isFollowing = false

            case .idle:
                isFollowing = isAtEnd

            default:
                break
            }
        }
        .onChange(of: snapshot.items.last) {
            if isFollowing {
                proxy.scrollTo(Self.endId, anchor: .bottom)
            }
        }
        .onChange(of: snapshot.anchor) { _, anchor in
            // only while the list is still at its top, where the page went in: a scroll away since is the user's
            if let anchor, isAtTop {
                proxy.scrollTo(anchor, anchor: .top)
            }
        }
        .task(id: snapshot.toEnd) {
            guard snapshot.toEnd else {
                return
            }
            proxy.scrollTo(Self.endId, anchor: .bottom)
            isFollowing = true
            onEndShown()
        }
    }

    // the room around a row: a reply breathes, the one-line rows sit close (a prompt keeps its own)
    private func gap(_ item: ChatItem) -> CGFloat {
        switch item.body {
        case .assistant:
            Spacing.xSmall

        default:
            0
        }
    }
}
