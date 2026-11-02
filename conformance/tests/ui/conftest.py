"""Selectors shared by UI tests; platform drivers expose the tree without app-specific aliases."""

import pytest
from agterm_conformance.wait import wait_until


@pytest.fixture
def palette_hint(ui):
    """Read the membership editor's hint, including SwiftUI's inherited parent identifier.

    v0.39.1 Palette.swift gives Text its own id, but macOS AX reports command-palette on that text.
    The inherited-id candidate must be text with the hint's prefix, never the search field or a result row.
    """
    def read():
        def find():
            for element in ui.all("palette-editor-hint"):
                if element.role == "text":
                    return ui.text(element)
            for element in ui.all("command-palette"):
                if element.role == "text" and ui.text(element).startswith("Return toggles"):
                    return ui.text(element)
            return None

        return wait_until(find, timeout=ui.timeout)

    return read
