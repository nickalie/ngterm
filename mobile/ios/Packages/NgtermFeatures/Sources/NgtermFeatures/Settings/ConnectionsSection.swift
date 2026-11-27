import NgtermPlatform
import SwiftUI

/// The connections (mobile/android's `ConnectionsSection`): how long a connect may take to reach a machine.
internal struct ConnectionsSection: View {
    internal let model: SettingsModel

    internal var body: some View {
        let seconds = model.settings.connectTimeout
        Section {
            NumberSlider(
                label: String(localized: "Connection timeout"),
                value: seconds,
                range: AppSettings.connectTimeouts,
                shown: String(localized: "\(seconds) s"),
                spoken: String(localized: "\(seconds) seconds"),
                note: String(
                    localized: """
                        How long a machine may take to answer a connect. A machine's own timeout, in its form, wins. \
                        It applies from the next connect.
                        """
                ),
                identifier: "connect-timeout"
            ) { picked in
                model.change { $0.connectTimeout = picked }
            }
        } header: {
            FormHint(String(localized: "Connections"), .header)
        }
    }
}
