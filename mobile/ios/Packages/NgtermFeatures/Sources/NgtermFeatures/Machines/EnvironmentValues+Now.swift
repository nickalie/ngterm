import Foundation
import SwiftUI

extension EnvironmentValues {
    /// The time a reconnect's countdown is told from, when it is fixed (a golden's); `nil` follows the clock.
    @Entry internal var fixedNow: Date?
}
