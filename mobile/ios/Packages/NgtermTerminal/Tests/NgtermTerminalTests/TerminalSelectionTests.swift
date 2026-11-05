import NgtermCoreFakes
import NgtermTerminal
import Testing
import UIKit

/// The selection's side of the view against the fake pane: copy, paste and its confirmation, select all, and when a
/// touch selection is over.
@MainActor
@Suite("the terminal's selection and clipboard")
internal struct TerminalSelectionTests {
    @Test("copy puts the selection's text on the clipboard and lets the selection go; with none it copies nothing")
    internal func copy() {
        let rig = TerminalRig(clipboard: "before")
        #expect(!rig.view.canPerformAction(#selector(UIResponder.copy(_:)), withSender: nil))
        rig.view.copy(nil)
        #expect(rig.clipboard.held == "before")
        #expect(rig.inputs.isEmpty)
        rig.pane.selection = "hello"
        #expect(rig.view.canPerformAction(#selector(UIResponder.copy(_:)), withSender: nil))
        rig.view.copy(nil)
        #expect(rig.clipboard.held == "hello")
        #expect(rig.inputs == [.clearSelection])
    }

    @Test("paste sends the clipboard's text, and an empty clipboard sends nothing")
    internal func paste() {
        let rig = TerminalRig(clipboard: nil)
        #expect(!rig.view.canPerformAction(#selector(UIResponder.paste(_:)), withSender: nil))
        rig.view.paste(nil)
        #expect(rig.inputs.isEmpty)
        rig.clipboard.held = "ls -la"
        #expect(rig.view.canPerformAction(#selector(UIResponder.paste(_:)), withSender: nil))
        rig.view.paste(nil)
        #expect(rig.inputs == [.paste(text: "ls -la", allowUnsafe: false)])
        #expect(rig.view.selection.unsafePaste == nil)
    }

    @Test("paste confirms text that can run a command: yes pastes it as it is, no drops it")
    internal func pasteConfirms() {
        let rig = TerminalRig(clipboard: "rm -rf build\n")
        let selection = rig.view.selection
        rig.view.paste(nil)
        #expect(selection.unsafePaste == "rm -rf build\n")
        selection.confirmPaste()
        #expect(
            rig.inputs == [
                .paste(text: "rm -rf build\n", allowUnsafe: false),
                .paste(text: "rm -rf build\n", allowUnsafe: true),
            ]
        )
        #expect(selection.unsafePaste == nil)
        selection.paste("make\n")
        selection.cancelPaste()
        #expect(selection.unsafePaste == nil)
        selection.confirmPaste()
        #expect(rig.inputs.count == 3)
    }

    @Test("a paste over a touch selection lets it go first")
    internal func pasteOverSelection() {
        let rig = TerminalRig(clipboard: "word")
        rig.view.gestures.longPress(.began, at: rig.point(1, 0))
        rig.view.gestures.longPress(.ended, at: rig.point(1, 0))
        rig.view.paste(nil)
        #expect(
            rig.inputs == [
                .selectWord(TerminalRig.cell(1, 0)),
                .clearSelection,
                .paste(text: "word", allowUnsafe: false),
            ]
        )
        #expect(!rig.view.selection.isTouchSelection)
    }

    @Test("select all selects everything, and a touch selection shows its menu again over it")
    internal func selectAll() {
        let rig = TerminalRig(clipboard: nil)
        #expect(rig.view.canPerformAction(#selector(UIResponder.selectAll(_:)), withSender: nil))
        rig.view.gestures.longPress(.began, at: rig.point(1, 0))
        rig.show([SelectedSpan(row: 0, first: 0, last: 4)])
        rig.view.gestures.longPress(.ended, at: rig.point(1, 0))
        rig.view.selectAll(nil)
        #expect(rig.inputs.last == .selectAll)
        #expect(rig.view.selection.wantsMenu)
        rig.show([SelectedSpan(row: 0, first: 0, last: 10), SelectedSpan(row: 3, first: 0, last: 6)])
        #expect(rig.view.isMenuShown)
        #expect(rig.view.handleSpots?.end.cell == TerminalRig.cell(6, 3))
    }

    @Test("a touch selection is over once a frame shows it gone, not before its first frame shows it")
    internal func touchSelectionEnds() {
        let rig = TerminalRig(clipboard: nil)
        rig.view.gestures.tap(at: rig.point(1, 0), time: 1)
        rig.view.gestures.tap(at: rig.point(1, 0), time: 1.1)
        rig.show([])
        #expect(rig.view.selection.isTouchSelection)
        rig.show([SelectedSpan(row: 0, first: 0, last: 4)])
        #expect(rig.view.handleSpots != nil)
        // output cleared it
        rig.show([])
        #expect(!rig.view.selection.isTouchSelection)
        #expect(rig.view.handleSpots == nil)
    }

    @Test("a pane with no terminal drops the selection's calls")
    internal func noTerminal() {
        let rig = TerminalRig(clipboard: "text\n")
        rig.pane.frames = nil
        rig.view.gestures.longPress(.began, at: rig.point(1, 0))
        rig.view.paste(nil)
        #expect(rig.inputs.isEmpty)
        #expect(!rig.view.selection.isTouchSelection)
        #expect(rig.view.selection.unsafePaste == nil)
    }
}
