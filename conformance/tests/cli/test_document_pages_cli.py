"""Local document source paths and early CLI conflicts for image and Markdown overlays."""

import base64
import os
import uuid

import pytest
from agterm_conformance.match import assert_ok, find_session


@pytest.mark.since("0.40.0")
@pytest.mark.covers("cli:session overlay open", "session.overlay.open")
@pytest.mark.parametrize("source,filename,contents,field", [
    ("--image", "image.png", base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="), "image"),
    ("--markdown", "notes.md", b"# Local document\n\n**markdown** text", "markdown"),
], ids=["image", "markdown"])
def test_cli_document_sources_resolve_relative_paths(cli, ctl, source, filename, contents, field):
    sid = assert_ok(ctl.request("session.new"))["id"]
    ctl.wait_mounted(sid, "left")
    path = ctl.instance.home / f"document-{uuid.uuid4().hex}-{filename}"
    path.write_bytes(contents)
    try:
        result = cli("session", "overlay", "open", source, os.fspath(path.name), "--target", sid)
        assert (result.returncode, result.stderr) == (0, ""), (result.stdout, result.stderr)
        node = find_session(ctl.tree(), sid)
        overlay = node["htmlOverlays"][0]
        assert overlay[field] == os.fspath(path)
        assert not ({"file", "url", "image", "markdown"} - {field}) & overlay.keys()
    finally:
        assert_ok(ctl.request("session.close", sid))
        path.unlink(missing_ok=True)


@pytest.mark.since("0.40.0")
@pytest.mark.covers("cli:session overlay open")
@pytest.mark.parametrize("source", ["--image", "--markdown"])
def test_cli_document_sources_reject_incompatible_flags_before_connecting(cli, source):
    for flag in ["--companion", "--block", "--wait", "--navigation", "--js", "--chromeless", "--persistent", "--browse"]:
        result = cli("session", "overlay", "open", source, "local-file", flag)
        assert (result.returncode, result.stdout) == (64, ""), result
        assert result.stderr.startswith("Error: "), result.stderr
        assert f"\nUsage: {result.program} session overlay open " in result.stderr
    result = cli("session", "overlay", "open", "--image", "image.png", "--markdown", "doc.md")
    assert (result.returncode, result.stdout) == (64, ""), result
    assert "provide exactly one of COMMAND, --html, --url, --image or --markdown" in result.stderr
