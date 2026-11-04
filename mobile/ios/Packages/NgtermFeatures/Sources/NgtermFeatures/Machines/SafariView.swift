import SafariServices
import SwiftUI

/// Safari's view of a web page, in the app.
///
/// A sign-in link opens here, so the app stays in the foreground. Its Done calls `onDone`, which closes the sheet it
/// is in.
internal struct SafariView: UIViewControllerRepresentable {
    /// Hands Safari's Done to the sheet, on the main actor, where Safari's view calls it.
    @MainActor
    internal final class Coordinator: NSObject, @MainActor SFSafariViewControllerDelegate {
        private let onDone: @MainActor () -> Void

        internal init(onDone: @escaping @MainActor () -> Void) {
            self.onDone = onDone
        }

        internal func safariViewControllerDidFinish(_: SFSafariViewController) {
            onDone()
        }

        deinit {
            // nothing to release
        }
    }

    internal let url: URL
    internal let onDone: @MainActor () -> Void

    internal func makeCoordinator() -> Coordinator {
        Coordinator(onDone: onDone)
    }

    internal func makeUIViewController(context: Context) -> SFSafariViewController {
        let controller = SFSafariViewController(url: url)
        controller.dismissButtonStyle = .done
        controller.delegate = context.coordinator
        return controller
    }

    internal func updateUIViewController(_: SFSafariViewController, context _: Context) {
        // the page is the one it opened with
    }
}
