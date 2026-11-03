import SwiftUI

@main
struct ProbeApp: App {
    var body: some Scene {
        WindowGroup {
            ProbeView()
        }
    }
}

struct ProbeView: View {
    @State private var taps = 0

    var body: some View {
        VStack(spacing: 16) {
            Text("taps: \(taps)")
                .accessibilityIdentifier("count")
            Button("Tap") { taps += 1 }
                .accessibilityIdentifier("tap")
        }
    }
}
