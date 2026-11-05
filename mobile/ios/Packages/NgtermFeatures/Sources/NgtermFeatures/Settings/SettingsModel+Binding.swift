import NgtermPlatform
import SwiftUI

extension SettingsModel {
    /// The setting at `path` as a binding a control changes it through: each change is written as it is made.
    internal func binding<Value>(_ path: WritableKeyPath<AppSettings, Value>) -> Binding<Value> {
        Binding {
            self.settings[keyPath: path]
        } set: { value in
            self.change { $0[keyPath: path] = value }
        }
    }

    /// The notifications of `kind` as a switch's binding.
    internal func noticeBinding(_ kind: NoticeKind) -> Binding<Bool> {
        Binding {
            self.enabledNotices.contains(kind)
        } set: { isOn in
            self.setNotice(kind, isOn: isOn)
        }
    }
}
