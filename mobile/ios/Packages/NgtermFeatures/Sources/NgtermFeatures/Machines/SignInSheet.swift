import Foundation
import NgtermDesign
import NgtermPlatform
import SwiftUI

/// `machine` asks the user to sign in at `url` before it lets the connect in (`message` is what it said, Tailscale
/// SSH's `check` say).
///
/// Open link, or a tap on the link in the message, opens it in the app (Safari's view over the sheet), so the app
/// stays in the foreground while the user signs in; Copy link puts it on the clipboard. The sheet waits, and goes
/// once the connect ends either way. Don't connect, or swiping the sheet away, cancels the connect.
internal struct SignInSheet: View {
    internal let machine: StoredMachine
    internal let message: String
    internal let url: String
    internal let onCancel: @MainActor () -> Void
    @State private var page: SafariPage?

    internal var body: some View {
        NavigationStack {
            PromptPage(
                tone: .attention,
                title: String(localized: "\(machine.title) asks you to sign in"),
                machine: machine,
                message: String(
                    localized: "Open the link and sign in there. Once you have, the connection goes on by itself."
                )
            ) {
                ValuePlate(String(localized: "Message from \(machine.title)"), text: linked) {
                    CopyButton(url, title: String(localized: "Copy link"))
                }
                .environment(
                    \.openURL,
                    OpenURLAction { opened in
                        open(opened)
                        return .handled
                    }
                )
                HStack(spacing: Spacing.small) {
                    ProgressView()
                    Text("Waiting for you to sign in")
                        .font(Typography.note)
                        .foregroundStyle(Palette.secondaryText.color)
                }
                .accessibilityElement(children: .combine)
                actions
            }
            .navigationBarTitleDisplayMode(.inline)
        }
        .sheet(item: $page) { shown in
            SafariView(url: shown.url) {
                page = nil
            }
            .ignoresSafeArea()
        }
    }

    private var actions: some View {
        VStack(spacing: Spacing.small) {
            Button {
                if let link = URL(string: url) {
                    open(link)
                }
            } label: {
                Text("Open link")
                    .frame(maxWidth: .infinity)
            }
            .prominentAction()
            .accessibilityIdentifier("open-sign-in")
            Button {
                onCancel()
            } label: {
                Text("Don't connect")
                    .frame(maxWidth: .infinity, minHeight: Spacing.hitTarget)
            }
            .accessibilityIdentifier("cancel-sign-in")
        }
        .padding(.top, Spacing.small)
    }

    // the message with the link in it as a link; a message that doesn't show the link gets it under it
    private var linked: Text {
        let shown = message.contains(url) ? message : [message, url].filter { !$0.isEmpty }.joined(separator: "\n")
        var text = AttributedString(shown)
        if let range = text.range(of: url), let link = URL(string: url) {
            text[range].link = link
            text[range].underlineStyle = .single
        }
        return Text(text)
    }

    // only a web page opens in the app's Safari view; anything else stays on the sheet to copy
    private func open(_ link: URL) {
        if SafariPage.canShow(link) {
            page = SafariPage(url: link)
        }
    }
}
