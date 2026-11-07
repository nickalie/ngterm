import NgtermCore

/// The answers a question card holds before they are sent (mobile/android's `QuestionDraft`), one slot per question of
/// the `AskUserQuestion`: the option labels `picked`, the text typed into its "Type an answer" row (`other`), and
/// whether that row is picked (`otherPicked`).
///
/// A single-select question holds one pick: an option, or the typed text, which typing picks and clearing lets go. A
/// multi-select question holds any options, and the typed text with them once it is picked.
public struct QuestionDraft: Equatable, Sendable {
    /// Each question's picked option labels.
    public private(set) var picked: [[String]]
    /// Each question's typed text.
    public private(set) var other: [String]
    /// Whether each question's typed row is picked.
    public private(set) var otherPicked: [Bool]

    /// An empty draft for `question`.
    public init(for question: ChatQuestion) {
        let count = question.questions.count
        picked = Array(repeating: [], count: count)
        other = Array(repeating: "", count: count)
        otherPicked = Array(repeating: false, count: count)
    }

    private static func isBlank(_ text: String) -> Bool {
        text.allSatisfy(\.isWhitespace)
    }

    /// The draft with option `label` of question `index` picked, or let go when it is a multi-select pick already.
    public func toggling(_ index: Int, _ item: ChatQuestionItem, label: String) -> Self {
        var next = self
        let now = picked[index]
        if !item.multiSelect {
            next.picked[index] = [label]
            next.otherPicked[index] = false
        } else if now.contains(label) {
            next.picked[index] = now.filter { $0 != label }
        } else {
            next.picked[index] = now + [label]
        }
        return next
    }

    /// The draft with `text` typed for question `index`: typing picks the row, clearing it lets the row go.
    public func typing(_ index: Int, _ item: ChatQuestionItem, text: String) -> Self {
        var next = self
        let typed = !Self.isBlank(text)
        if typed, !item.multiSelect {
            next.picked[index] = []
        }
        next.other[index] = text
        next.otherPicked[index] = typed
        return next
    }

    /// The draft with the typed row of question `index` picked or let go; it takes a tap only while it holds text.
    public func togglingOther(_ index: Int, _ item: ChatQuestionItem) -> Self {
        guard !Self.isBlank(other[index]) else {
            return self
        }
        var next = self
        let isOn = !otherPicked[index] || !item.multiSelect
        if isOn, !item.multiSelect {
            next.picked[index] = []
        }
        next.otherPicked[index] = isOn
        return next
    }

    /// The answers to send for `question`, one per question in order; none while one has none.
    ///
    /// A single-select question takes exactly one answer, a multi-select one at least one.
    public func answers(for question: ChatQuestion) -> [QuestionAnswer] {
        var answers: [QuestionAnswer] = []
        for index in question.questions.indices {
            let trimmed = other[index].trimmingCharacters(in: .whitespacesAndNewlines)
            let text = otherPicked[index] && !trimmed.isEmpty ? trimmed : nil
            guard !picked[index].isEmpty || text != nil else {
                return []
            }
            answers.append(QuestionAnswer(picked: picked[index], other: text))
        }
        return answers
    }

    /// Whether the draft can stand for `question`: one slot for each of its questions, each pick one of its options.
    public func fits(_ question: ChatQuestion) -> Bool {
        let items = question.questions
        guard picked.count == items.count, other.count == items.count, otherPicked.count == items.count else {
            return false
        }
        return items.indices.allSatisfy { index in
            let labels = Set(items[index].options.map(\.label))
            return picked[index].allSatisfy(labels.contains)
        }
    }
}
