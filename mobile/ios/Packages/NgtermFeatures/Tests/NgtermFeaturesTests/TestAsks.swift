import NgtermCore

/// Asks for the tests (mobile/android's `testAsk` and `testChoices`).
internal enum TestAsks {
    /// An ask `A1`, "Deploy to production?": `buttons` by id (labelled the same), the first the default and `no` the
    /// cancel button, with `input` or `choices` when given.
    internal static func ask(
        _ buttons: [String],
        input: RemoteAskInput?,
        choices: RemoteAskChoices?
    ) -> RemoteAsk {
        RemoteAsk(
            id: "A1",
            title: "Deploy to production?",
            message: nil,
            buttons: buttons.map { RemoteAskButton(id: $0, label: $0, hotkey: nil) },
            defaultId: buttons.first,
            destructiveId: nil,
            cancelId: "no",
            style: "terminal",
            align: "right",
            width: nil,
            pane: nil,
            input: input,
            choices: choices
        )
    }

    /// ``ask(_:input:choices:)`` with Yes and No.
    internal static func plain() -> RemoteAsk {
        ask(["yes", "no"], input: nil, choices: nil)
    }

    /// ``ask(_:input:choices:)`` with Yes and No and `input`.
    internal static func asking(_ input: RemoteAskInput) -> RemoteAsk {
        ask(["yes", "no"], input: input, choices: nil)
    }

    /// ``ask(_:input:choices:)`` with Yes and No and `list`.
    internal static func choosing(_ list: RemoteAskChoices) -> RemoteAsk {
        ask(["yes", "no"], input: nil, choices: list)
    }

    /// A choice list of a, b and c (labelled A, B and C), `multiple` or not, with `checked` checked and the free-form
    /// row `other` when given.
    internal static func choices(multiple: Bool, checked: [String], other: String?) -> RemoteAskChoices {
        RemoteAskChoices(
            choices: ["a", "b", "c"].map { RemoteAskChoice(id: $0, label: $0.uppercased(), subtitle: nil) },
            multiple: multiple,
            checked: checked,
            other: other
        )
    }
}
