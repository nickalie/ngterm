import NgtermDesign
import NgtermPlatform
import SwiftUI
import UIKit

/// The notification kinds' switches; and while the app may not notify, a banner that turns notifications on: it asks
/// the system's question when the user hasn't answered it, and opens the system's settings for the app once they
/// said no (only there can it be undone).
internal struct NotificationsSection: View {
    @Environment(\.openURL)
    private var openURL

    internal let model: SettingsModel

    internal var body: some View {
        Section {
            if let banner {
                banner
            }
            ForEach(SettingsChoices.notices, id: \.value) { choice in
                Toggle(isOn: model.noticeBinding(choice.value)) {
                    ChoiceLabel(choice.title, note: choice.note)
                }
                .accessibilityIdentifier("notice-\(choice.value.storedName)")
            }
        } header: {
            FormHint(String(localized: "Notifications"), .header)
        } footer: {
            FormHint(String(localized: "Questions from agents always notify: they wait for your answer."), .note)
        }
    }

    private var banner: Banner? {
        switch model.permission {
        case .undecided:
            Banner(
                .attention,
                message: String(localized: "ngterm can't notify you yet"),
                note: String(localized: "Turn notifications on to hear about agents while you're away."),
                action: RowAction(String(localized: "Turn on")) {
                    Task {
                        await model.askPermission()
                    }
                }
            )

        case .denied:
            Banner(
                .attention,
                message: String(localized: "Notifications are off for ngterm"),
                note: String(localized: "Turn them on in the Settings app to hear about agents while you're away."),
                action: RowAction(String(localized: "Open Settings")) {
                    if let url = URL(string: UIApplication.openNotificationSettingsURLString) {
                        openURL(url)
                    }
                }
            )

        case .allowed, nil:
            nil
        }
    }
}
