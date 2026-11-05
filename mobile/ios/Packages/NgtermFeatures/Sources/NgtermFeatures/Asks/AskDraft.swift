import NgtermCore

/// The user's unsubmitted input to an ask (mobile/android's `AskDraft`, after ngterm-core's, upstream's): an input
/// field's ``text``, or a choice list's ``checked`` ids and its free-form row (its ``text``, ``isOtherSelected``).
///
/// A value, so the sheet keeps it as state and every change is a new draft. A radio list starts on its first checked
/// id, else its first item. In a choice list a non-blank text selects the free-form row (a radio list hands its pick
/// over, and gets it back when the text goes blank); a blank one unselects it, and an answer never carries blank
/// free-form text.
public struct AskDraft: Hashable, Sendable {
    /// The answer of a dismissed ask (a swipe, or its cancel button): no button, no payload; the origin escapes.
    public static let escaped = AskReply(button: nil, text: nil, selected: nil)

    /// The field's text, or the free-form row's.
    public private(set) var text: String
    /// The checked ids.
    public private(set) var checked: Set<String>
    /// Whether the free-form row is selected.
    public private(set) var isOtherSelected: Bool
    private let choiceIds: [String]
    private let isMultiple: Bool
    private let hasOther: Bool
    // the radio pick the free-form row took over, given back when its text goes blank
    private var previousPick: String?

    /// Whether the free-form row's text is not blank, so the row can be selected.
    public var isOtherEnabled: Bool {
        hasOther && !AskText.isBlank(text)
    }

    /// The checked ids in the ask's order; the free-form row answers through ``otherText``.
    public var selected: [String] {
        choiceIds.filter(checked.contains)
    }

    /// The free-form row's text while the row is selected.
    public var otherText: String? {
        isOtherSelected && isOtherEnabled ? text : nil
    }

    /// The draft `ask` opens with: its field's prefill, or its list's checked ids (a radio list's first).
    public init(_ ask: RemoteAsk) {
        let list = ask.choices
        let ids = list?.choices.map(\.id) ?? []
        if let list, list.multiple {
            checked = Set(list.checked)
        } else if let list {
            checked = Set([list.checked.first ?? ids.first].compactMap(\.self))
        } else {
            checked = []
        }
        text = AskText.clamp(ask.input?.value ?? "")
        isOtherSelected = false
        choiceIds = ids
        isMultiple = list?.multiple == true
        hasOther = list?.other != nil
        previousPick = nil
    }

    /// The answer of `buttonId` pressed on a notification of `ask` (mobile/android's `notificationReply`): the cancel
    /// button escapes; an input ask answers with the reply `typed` there, fitted to its field (a line field's line
    /// breaks become spaces), or its prefill when nothing was typed; anything else as the sheet would with the ask's
    /// initial values.
    public static func notificationReply(_ ask: RemoteAsk, button buttonId: String, typed: String?) -> AskReply {
        guard let input = ask.input, buttonId != ask.cancelId else {
            return Self(ask).reply(to: ask, button: buttonId)
        }
        let fitted = AskText.fit(typed ?? input.value, multiline: input.multiline)
        return AskReply(button: buttonId, text: fitted, selected: nil)
    }

    /// The text replaced by `value`, cut to the cap.
    ///
    /// In a choice list a non-blank text selects the free-form row and a blank one unselects it.
    public func withText(_ value: String) -> Self {
        var next = self
        next.text = AskText.clamp(value)
        guard hasOther else {
            return next
        }
        return AskText.isBlank(next.text) ? next.unselectingOther() : next.selectingOther()
    }

    /// Picks the radio item or flips the checkbox at `index`; the free-form row (the index after the choices) toggles
    /// only while enabled, and a picked radio row stays picked.
    public func toggle(_ index: Int) -> Self {
        if hasOther, index == choiceIds.count {
            return togglingOther()
        }
        guard choiceIds.indices.contains(index) else {
            return self
        }
        let id = choiceIds[index]
        var next = self
        if !isMultiple {
            next.checked = [id]
            next.isOtherSelected = false
            next.previousPick = nil
        } else if checked.contains(id) {
            next.checked.remove(id)
        } else {
            next.checked.insert(id)
        }
        return next
    }

    /// The answer of pressing `buttonId` on `ask` with this draft (ngterm-core's `PendingAsk::answer`): the cancel
    /// button escapes with no payload; any other answers with the field's text, or the selected ids and the selected
    /// free-form row's text.
    public func reply(to ask: RemoteAsk, button buttonId: String) -> AskReply {
        if ask.cancelId == buttonId {
            return Self.escaped
        }
        if ask.input != nil {
            return AskReply(button: buttonId, text: text, selected: nil)
        }
        if ask.choices != nil {
            return AskReply(button: buttonId, text: otherText, selected: selected)
        }
        return AskReply(button: buttonId, text: nil, selected: nil)
    }

    private func togglingOther() -> Self {
        guard isOtherEnabled else {
            return self
        }
        guard isMultiple, isOtherSelected else {
            return selectingOther()
        }
        var next = self
        next.isOtherSelected = false
        return next
    }

    private func selectingOther() -> Self {
        guard !isOtherSelected else {
            return self
        }
        var next = self
        next.isOtherSelected = true
        if !isMultiple {
            next.previousPick = checked.first
            next.checked = []
        }
        return next
    }

    private func unselectingOther() -> Self {
        guard isOtherSelected else {
            return self
        }
        var next = self
        next.isOtherSelected = false
        if !isMultiple {
            next.checked = Set([previousPick ?? choiceIds.first].compactMap(\.self))
            next.previousPick = nil
        }
        return next
    }
}
