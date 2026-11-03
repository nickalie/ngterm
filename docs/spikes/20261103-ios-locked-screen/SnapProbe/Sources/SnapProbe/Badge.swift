import SwiftUI

public struct Badge: View {
    public init() {}

    public var body: some View {
        Label("waiting", systemImage: "hourglass")
            .padding(12)
            .background(.orange.opacity(0.2), in: .capsule)
    }
}
