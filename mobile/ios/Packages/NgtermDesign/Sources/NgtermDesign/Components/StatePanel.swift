import SwiftUI

/// What a screen shows in place of its content: the system's `ContentUnavailableView`.
///
/// A glyph, a title, what to do, and the one action that does it, with at most one quieter action under it (a retry
/// after the fix the main one points to). An empty screen invites the first step; an error says what happened and how
/// to fix it, without apology.
public struct StatePanel: View {
    private let title: String
    private let systemImage: String?
    private let imageColor: ColorToken?
    private let message: String?
    private let action: RowAction?
    private let secondaryAction: RowAction?

    /// The body of the view.
    public var body: some View {
        // at the largest text sizes the panel may not fit: it scrolls then, rather than clip (the system's panel takes
        // whatever height it is offered, so it is offered at least the screen's and centers in it)
        GeometryReader { proxy in
            ScrollView {
                panel
                    .frame(minHeight: proxy.size.height)
            }
            .scrollBounceBehavior(.basedOnSize)
        }
    }

    private var panel: some View {
        ContentUnavailableView {
            // the glyph and the title apart, not a Label: the title is then an element of its own, and the
            // accessibility audit measures its text, not the glyph's thin strokes beside it
            VStack(spacing: Spacing.medium) {
                if let systemImage {
                    Image(systemName: systemImage)
                        .font(.largeTitle)
                        .imageScale(.large)
                        .foregroundStyle((imageColor ?? Palette.idle).color)
                        .accessibilityHidden(true)
                } else {
                    ProgressView()
                        .controlSize(.large)
                }
                Text(title)
                    .font(.title2.bold())
                    .accessibilityAddTraits(.isHeader)
            }
        } description: {
            if let message {
                // the app's secondary text: the system's grey stays under 4.5:1 on a sidebar's ground
                Text(message)
                    .foregroundStyle(Palette.secondaryText.color)
            }
        } actions: {
            if let action {
                Button(action.title, action: action.perform)
                    .prominentAction()
                    .disabled(!action.isEnabled)
            }
            if let secondaryAction {
                QuietButton(action: secondaryAction)
            }
        }
    }

    /// A panel titled `title`, with `systemImage` (a spinner for `nil`) in `imageColor`, its `message` and its actions.
    ///
    /// The glyph is ``Palette/idle`` when `imageColor` is `nil`: it reads on every ground as text does.
    public init(
        title: String,
        systemImage: String?,
        imageColor: ColorToken? = nil,
        message: String? = nil,
        action: RowAction? = nil,
        secondaryAction: RowAction? = nil
    ) {
        self.title = title
        self.systemImage = systemImage
        self.imageColor = imageColor
        self.message = message
        self.action = action
        self.secondaryAction = secondaryAction
    }

    /// The loading state: a spinner and what is under way ("Reading build-01's sessions").
    public static func loading(_ title: String, message: String? = nil) -> Self {
        Self(title: title, systemImage: nil, message: message)
    }

    /// The empty state: `systemImage`, `title`, the invitation (`message`) and the first step (`action`).
    public static func empty(systemImage: String, title: String, message: String, action: RowAction?) -> Self {
        Self(title: title, systemImage: systemImage, message: message, action: action)
    }

    /// The error state: the failed triangle, what happened (`title`), how to fix it (`message`) and the action that
    /// does, with an optional quieter one under it.
    public static func error(
        _ title: String,
        message: String,
        action: RowAction?,
        secondaryAction: RowAction? = nil
    ) -> Self {
        Self(
            title: title,
            systemImage: "exclamationmark.triangle.fill",
            imageColor: Palette.failed,
            message: message,
            action: action,
            secondaryAction: secondaryAction
        )
    }

    /// The offline state: the device has no network, and `retry` tries again.
    public static func offline(retry: RowAction) -> Self {
        Self(
            title: String(localized: "You're offline", bundle: .module),
            systemImage: "wifi.slash",
            imageColor: Palette.offline,
            message: String(
                localized: "This device has no network. Connect to Wi-Fi or cellular data, then try again.",
                bundle: .module
            ),
            action: retry
        )
    }
}
