import Foundation

/// A web page the app opens in Safari's view: an `http` or `https` link, which is all that view can show.
internal struct SafariPage: Hashable, Identifiable {
    internal let url: URL

    internal var id: URL {
        url
    }

    /// Whether Safari's view can show `url`.
    internal static func canShow(_ url: URL) -> Bool {
        ["http", "https"].contains(url.scheme?.lowercased())
    }
}
