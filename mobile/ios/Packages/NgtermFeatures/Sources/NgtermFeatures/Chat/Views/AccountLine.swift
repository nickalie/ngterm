import NgtermDesign
import SwiftUI

/// The account the agent is signed in with and how much of its weekly limit is used (mobile/android's `AccountLine`).
///
/// One quiet line over the chat's rows, with no rule or meter: the email at the start; at the end "Weekly limit 72%",
/// with ", resets Mon 14:00" after it when the origin read when, taking the attention copper from 80% and oxide once
/// spent. When the two don't fit on one line the limit goes under the email. VoiceOver reads it as one sentence.
internal struct AccountLine: View {
    private static let high: UInt32 = 80
    private static let full: UInt32 = 100
    // within this many seconds a reset reads as a weekday: six days
    private static let weekdaysAhead: TimeInterval = 518_400

    internal let account: ChatAccount
    internal var now = Date()

    internal var body: some View {
        let resetWords = account.weekly?.resetsAt.map { seconds in
            Self.resetsAt(Date(timeIntervalSince1970: Double(seconds)), now: now, calendar: .current)
        }
        ViewThatFits(in: .horizontal) {
            HStack(spacing: Spacing.medium) {
                email
                Spacer(minLength: 0)
                limit(resetWords)
            }
            VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                email
                limit(resetWords)
            }
        }
        .font(.footnote)
        .padding(.vertical, Spacing.xSmall)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(spoken(resetWords))
    }

    @ViewBuilder private var email: some View {
        if let address = account.email {
            Text(verbatim: address)
                .foregroundStyle(Palette.secondaryText.color)
                .lineLimit(1)
                .truncationMode(.middle)
        }
    }

    /// When a limit resets (`moment`), as of `now` in `calendar`.
    ///
    /// The short day and the locale's short time within six days ("Mon 14:00"), else the locale's medium date ("Nov
    /// 14, 2026").
    internal static func resetsAt(_ moment: Date, now: Date, calendar: Calendar) -> String {
        if moment.timeIntervalSince(now) <= weekdaysAhead {
            let day = moment.formatted(Date.FormatStyle(calendar: calendar).weekday(.abbreviated))
            let time = moment.formatted(Date.FormatStyle(date: .omitted, time: .shortened, calendar: calendar))
            return "\(day) \(time)"
        }
        return moment.formatted(Date.FormatStyle(date: .abbreviated, time: .omitted, calendar: calendar))
    }

    @ViewBuilder
    private func limit(_ resets: String?) -> some View {
        if let weekly = account.weekly {
            let used = min(weekly.usedPercent, Self.full)
            Text(
                resets.map { String(localized: "Weekly limit \(used)%, resets \($0)") }
                    ?? String(localized: "Weekly limit \(used)%")
            )
            .foregroundStyle(tone(used))
            .lineLimit(1)
        }
    }

    // quiet while there is room, the attention copper from 80%, oxide once spent
    private func tone(_ used: UInt32) -> Color {
        if used >= Self.full {
            return Palette.failed.color
        }
        return used >= Self.high ? Palette.attention.color : Palette.secondaryText.color
    }

    private func spoken(_ resets: String?) -> String {
        var parts: [String] = []
        if let address = account.email {
            parts.append(String(localized: "Signed in as \(address)."))
        }
        if let weekly = account.weekly {
            let used = String(localized: "\(min(weekly.usedPercent, Self.full))% of the weekly limit used")
            parts.append(resets.map { String(localized: "\(used), resets \($0).") } ?? "\(used).")
        }
        return parts.joined(separator: " ")
    }
}
