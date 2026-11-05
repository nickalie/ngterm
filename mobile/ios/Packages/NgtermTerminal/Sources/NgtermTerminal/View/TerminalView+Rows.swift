import UIKit

extension TerminalView {
    /// Shows a touch selection's handles where the screen has it, and the edit menu over it once it is wanted.
    internal func refreshHandles() {
        let spots = selection.isTouchSelection ? HandleSpots(screen.selection, font: font, size: bounds.size) : nil
        handles.show(spots, color: palette.cursor.uiColor.cgColor)
        guard let spots else {
            // the touch selection went: its menu goes with it, a secondary click's stays
            if !selection.isTouchSelection {
                editMenu.hideOverSelection()
            }
            return
        }
        if selection.wantsMenu, !selection.isAdjusting {
            selection.wantsMenu = false
            editMenu.show(over: spots.rows)
        }
    }

    /// Shows the edit menu at `point` (a secondary click).
    internal func showMenu(at point: CGPoint) {
        editMenu.show(at: point)
    }

    /// Takes the edit menu down.
    internal func hideMenu() {
        editMenu.hide()
    }

    // a layer per row of the screen, each a cell high across the bounds, at the screen's scale
    internal func layoutRows() {
        while rowLayers.count < screen.rowCount {
            let row = TerminalRowLayer()
            row.index = rowLayers.count
            row.view = self
            layer.insertSublayer(row, below: textInput.layer)
            rowLayers.append(row)
        }
        while rowLayers.count > screen.rowCount {
            rowLayers.removeLast().removeFromSuperlayer()
        }
        for row in rowLayers {
            if row.contentsScale != scale {
                row.contentsScale = scale
                row.setNeedsDisplay()
            }
            row.frame = CGRect(x: 0, y: font.top(row.index), width: bounds.width, height: font.cellHeight)
        }
    }

    /// Marks `rows` for drawing.
    internal func redraw(rows: some Sequence<Int>) {
        for index in rows where rowLayers.indices.contains(index) {
            rowLayers[index].setNeedsDisplay()
        }
    }

    /// Marks every row for drawing.
    internal func redrawAll() {
        redraw(rows: rowLayers.indices)
    }

    // asks before pasting `text`, which could run a command, in an alert over whatever the window shows
    internal func confirmPaste(_ text: String) {
        let alert = PasteConfirmation.alert(for: text) { [weak self] in
            self?.selection.confirmPaste()
        } onCancel: { [weak self] in
            self?.selection.cancelPaste()
        }
        var presenter = window?.rootViewController
        while let shown = presenter?.presentedViewController {
            presenter = shown
        }
        presenter?.present(alert, animated: true)
    }

    /// Gives the pane the palette's colors.
    internal func sendColors() {
        // a pane with no terminal gives its next one the colors anyway
        try? pane.setColors(colors: palette.paneColors)
    }
}
