"""Local document source paths and early CLI conflicts for image, Markdown and text overlays."""

import base64
import os
import uuid

import pytest
from agterm_conformance.match import assert_ok, find_session

pytestmark = pytest.mark.since("0.40.0")


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


ONE_SOURCE = "provide exactly one of COMMAND, --html, --url, --image, --markdown or --text"
TEXT_OPTIONS = "--text takes no --companion, --block, --wait, --navigation, --js, --chromeless, --persistent, --browse or --cwd"


def _usage(result, message=None):
    assert (result.returncode, result.stdout) == (64, ""), result
    assert result.stderr.startswith(f"Error: {message}\n" if message else "Error: "), result.stderr
    assert f"\nUsage: {result.program} session overlay open " in result.stderr


@pytest.mark.covers("cli:session overlay open")
@pytest.mark.parametrize("source", ["--image", "--markdown", pytest.param("--text", marks=pytest.mark.since("0.41.0"))])
def test_cli_document_sources_reject_incompatible_flags_before_connecting(cli, source):
    for flag in ["--companion", "--block", "--wait", "--navigation", "--js", "--chromeless", "--persistent", "--browse"]:
        _usage(cli("session", "overlay", "open", source, "local-file", flag))
    result = cli("session", "overlay", "open", "--image", "image.png", "--markdown", "doc.md")
    assert (result.returncode, result.stdout) == (64, ""), result
    assert ONE_SOURCE in result.stderr
    if source != "--text":
        return
    # --companion needs a page source first; every other page or program flag is refused by --text itself
    _usage(cli("session", "overlay", "open", "--text", "a.go", "--companion"), "--companion requires --html or --url")
    for flag in [["--block"], ["--wait"], ["--navigation"], ["--js"], ["--chromeless"], ["--persistent"], ["--browse"],
                 ["--cwd", "/tmp"]]:
        _usage(cli("session", "overlay", "open", "--text", "a.go", *flag), TEXT_OPTIONS)
    for other in [["revdiff"], ["--html", "r.html"], ["--url", "http://localhost:1/"], ["--image", "a.png"],
                  ["--markdown", "r.md"]]:
        _usage(cli("session", "overlay", "open", *other, "--text", "a.go"), ONE_SOURCE)
    _usage(cli("session", "overlay", "open", "--text", "a.go", "--line", "0"), "--line must be 1 or more")
    _usage(cli("session", "overlay", "open", "revdiff", "--line", "3"), "--line requires --text")
    _usage(cli("session", "overlay", "open", "--line", "3", "--markdown", "r.md"), "--line requires --text")


@pytest.mark.since("0.41.0")
@pytest.mark.covers("cli:session overlay open", "session.overlay.open")
def test_cli_text_source_is_taken_literally(cli, ctl):
    folder = ctl.instance.home / f"text-{uuid.uuid4().hex}"
    (folder / "nested").mkdir(parents=True)
    source = folder / "a.go"
    source.write_text("".join(f"line {n}\n" for n in range(1, 21)), encoding="utf-8")
    sid = assert_ok(ctl.request("session.new"))["id"]
    ctl.wait_mounted(sid, "left")
    try:
        # FILE is never parsed as path:line: `a.go:12` names a file whose type is no text page
        result = cli("session", "overlay", "open", "--text", f"{folder.name}/a.go:12", "--target", sid)
        assert (result.returncode, result.stdout, result.stderr) == (
            1, "", "error: session.overlay.open: --text must be a source, config, text or log file\n"), result
        assert "htmlOverlays" not in find_session(ctl.tree(), sid)
        # a relative FILE is resolved against the working directory and standardized before it is sent
        result = cli("session", "overlay", "open", "--text", f"{folder.name}/nested/../absent.go", "--target", sid)
        absent = os.fspath(folder / "absent.go")
        assert (result.returncode, result.stdout, result.stderr) == (
            1, "", f"error: session.overlay.open: unreadable text file: {absent}\n"), result
        result = cli("session", "overlay", "open", "--text", f"{folder.name}/a.go", "--line", "7", "--target", sid)
        assert (result.returncode, result.stderr) == (0, ""), (result.stdout, result.stderr)
        overlay = find_session(ctl.tree(), sid)["htmlOverlays"][0]
        assert overlay["text"] == {"file": os.fspath(source), "language": "go", "line": 7}, overlay
        assert not {"file", "url", "image", "markdown"} & overlay.keys()
    finally:
        assert_ok(ctl.request("session.close", sid))
