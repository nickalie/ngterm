import NgtermCore
import NgtermDesign
import SwiftUI

/// A pick, a sheet's page (mobile/android's `PickContent`): its prompt as the heading ("Pick one" without one), the
/// search field with the origin's query in it, the rows the text shows (``PickItemRow``; "No matches" for none), and
/// Cancel.
///
/// The rows are ranked as desktop's picker ranks them (``Picks/rows(of:query:)``). A row's tap answers with it; Return
/// answers with the marked row: the origin's selection while the text is as it opened, else the first row, as each
/// edit moves it back to the first.
public struct PickQuestion: View {
    private static let width: Double = 520

    private let pick: RemotePick
    private let onAnswer: @MainActor (PickReply) -> Void
    @State private var query: String
    @State private var marked: Int

    /// The page.
    public var body: some View {
        let rows = Picks.rows(of: pick, query: query)
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.medium) {
                Text(verbatim: title)
                    .font(.title2.bold())
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.isHeader)
                    .accessibilityIdentifier("pick-title")
                field(rows)
                if rows.isEmpty {
                    Text("No matches")
                        .foregroundStyle(Palette.secondaryText.color)
                        .padding(.vertical, Spacing.medium)
                } else {
                    LazyVStack(spacing: 0) {
                        ForEach(Array(rows.enumerated()), id: \.offset) { offset, row in
                            PickItemRow(row: row, items: pick.items, isMarked: offset == marked) {
                                onAnswer(PickChoice.reply(row, items: pick.items))
                            }
                        }
                    }
                }
                Button {
                    onAnswer(.cancelled)
                } label: {
                    Text("Cancel")
                        .foregroundStyle(Palette.tint.color)
                        .frame(maxWidth: .infinity, minHeight: Spacing.hitTarget)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.borderless)
                .accessibilityIdentifier("pick-cancel")
            }
            .frame(maxWidth: Self.width, alignment: .leading)
            .padding(.horizontal, Spacing.xLarge)
            .padding(.vertical, Spacing.large)
            .frame(maxWidth: .infinity)
        }
        .background(Color(uiColor: .systemGroupedBackground))
    }

    private var title: String {
        pick.prompt.flatMap { AskText.isBlank($0) ? nil : $0 } ?? String(localized: "Pick one")
    }

    // a refused edit (a control character) leaves the text as it was; an edit marks the first row again
    private var search: Binding<String> {
        Binding {
            query
        } set: { typed in
            if !AskText.hasForbiddenControls(typed, multiline: false) {
                query = typed
                marked = 0
            }
        }
    }

    /// The pick `pick`; a row, Return or Cancel runs `onAnswer` with its answer.
    @preconcurrency
    public init(pick: RemotePick, onAnswer: @escaping @MainActor (PickReply) -> Void) {
        self.pick = pick
        self.onAnswer = onAnswer
        let opening = pick.query ?? ""
        _query = State(initialValue: opening)
        _marked = State(
            initialValue: PickChoice.initialRow(
                Picks.rows(of: pick, query: opening),
                items: pick.items,
                selection: pick.selection
            )
        )
    }

    private func field(_ rows: [PickRow]) -> some View {
        HStack(spacing: Spacing.small) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(Palette.secondaryText.color)
                .accessibilityHidden(true)
            TextField(String(localized: "Search"), text: search)
                .submitLabel(.go)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
                .onSubmit {
                    if rows.indices.contains(marked) {
                        onAnswer(PickChoice.reply(rows[marked], items: pick.items))
                    }
                }
                .accessibilityIdentifier("pick-search")
        }
        .padding(.horizontal, Spacing.medium)
        .padding(.vertical, Spacing.small)
        .frame(minHeight: Spacing.hitTarget)
        .background(
            Color(uiColor: .secondarySystemGroupedBackground),
            in: RoundedRectangle(cornerRadius: Radius.medium, style: .continuous)
        )
    }
}
