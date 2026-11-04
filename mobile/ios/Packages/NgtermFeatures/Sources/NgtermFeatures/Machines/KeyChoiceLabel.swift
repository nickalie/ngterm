import NgtermDesign
import NgtermPlatform
import SwiftUI

/// A key as the form offers it: where it came from, over its fingerprint.
internal struct KeyChoiceLabel: View {
    internal let key: AppKey

    internal var body: some View {
        VStack(alignment: .leading, spacing: Spacing.xxSmall) {
            Text(key.isImported ? String(localized: "Imported") : String(localized: "Created on this device"))
            Text(verbatim: key.fingerprint)
                .font(Typography.address)
                .foregroundStyle(Palette.secondaryText.color)
                .lineLimit(1)
                .truncationMode(.middle)
        }
    }
}
