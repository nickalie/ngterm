import importlib.util
import sys
from pathlib import Path

import pytest

TOOLS = Path(__file__).resolve().parent.parent / "tools"


def load(name: str):
    spec = importlib.util.spec_from_file_location(name, TOOLS / f"{name}.py")
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


# the probe reads the build's launch and log helpers from its sibling
load("ghostty_probe")
webkit_probe = load("webkit_probe")


def test_the_engine_lines_of_the_apps_log_are_kept_without_colour() -> None:
    log = (
        "\x1b[2m2026-10-17T00:00:00Z\x1b[0m \x1b[32m INFO\x1b[0m ngterm_gpui::html_overlay::registry: the web engine started\n"
        "INFO ngterm_gpui::app: quit: the state is saved\n"
        "WARN ngterm_gpui::external_texture: external frames fall back to shared memory: no render node\n"
    )
    lines = webkit_probe.engine_lines(log)
    assert lines == [
        "2026-10-17T00:00:00Z  INFO ngterm_gpui::html_overlay::registry: the web engine started",
        "WARN ngterm_gpui::external_texture: external frames fall back to shared memory: no render node",
    ]


def test_the_page_read_back_is_the_sessions_first() -> None:
    tree = {
        "tree": {
            "workspaces": [
                {
                    "id": "W",
                    "sessions": [
                        {"id": "A", "htmlOverlays": [{"state": "loaded", "title": "t"}]},
                        {"id": "B"},
                    ],
                }
            ]
        }
    }
    assert webkit_probe.page_state(tree, "A") == {"state": "loaded", "title": "t"}
    assert webkit_probe.page_state(tree, "B") is None


def test_a_missing_build_fails_without_launching(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.setenv("CONFORMANCE_LAVAPIPE_ICD", str(tmp_path / "lvp_icd.json"))
    report = webkit_probe.probe(tmp_path / "ngterm", 1)
    assert report["ok"] is False and "no ngterm at" in report["error"]
    assert report["engine"] == "wpe"


def test_the_probe_defaults_to_the_runners_stage(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    seen: list[Path] = []
    monkeypatch.setattr(webkit_probe, "probe", lambda bin, timeout: seen.append(bin) or {"ok": True})
    assert webkit_probe.main(["--bin", str(tmp_path / "ngterm")]) == 0
    assert webkit_probe.main([]) == 0
    assert seen == [(tmp_path / "ngterm").resolve(), webkit_probe.STAGE_BIN.resolve()]
    assert webkit_probe.STAGE_BIN.parts[-3:] == ("conformance-stage", "bin", "ngterm")


def test_a_page_that_reads_back_loaded_before_its_title_is_waited_for(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    """WebKit reports a load finished before the title its document set (the title is a task the document queues), so
    a read-back can find the page loaded without its title yet: the probe waits for the title, as the suite does."""
    from agterm_conformance.client import Response
    from agterm_conformance.impls import rust

    def page(**fields: object) -> Response:
        overlay = {"state": "loaded", "page": str(tmp_path / "webkit-probe.html"), **fields}
        tree = {"tree": {"workspaces": [{"id": "W", "sessions": [{"id": "S", "htmlOverlays": [overlay]}]}]}}
        return Response(True, tree, None, {})

    reads = iter([page(), page(), page(title=webkit_probe.TITLE)])

    class Instance:
        root = tmp_path
        log = tmp_path / "app.log"

        def request(self, method: str, *args: object, **params: object) -> Response:
            if method == "tree":
                return next(reads)
            return Response(True, {"id": "S"}, None, {})

    class Adapter:
        def __init__(self, bin: Path) -> None:
            pass

        def launch(self) -> Instance:
            return Instance()

        def stop(self, instance: Instance) -> None:
            pass

    (tmp_path / "app.log").write_text("")
    (tmp_path / "ngterm").write_text("")
    monkeypatch.setenv("CONFORMANCE_LAVAPIPE_ICD", str(tmp_path / "lvp_icd.json"))
    monkeypatch.setattr(rust, "RustAdapter", Adapter)
    monkeypatch.setattr(webkit_probe.time, "sleep", lambda _: None)
    report = webkit_probe.probe(tmp_path / "ngterm", 5)
    assert report["ok"] is True, report.get("error")
    assert report["page"]["title"] == webkit_probe.TITLE


def test_a_page_short_of_its_title_at_the_deadline_says_which_part_is_missing() -> None:
    assert webkit_probe.missing({"state": "loaded"}, 60) == (
        "the page loaded but did not show its title 'ngterm-webkit-probe' within 60s"
    )
    assert webkit_probe.missing({"state": "loading"}, 60) == "the page did not load within 60s"
    assert webkit_probe.missing(None, 60) == "the page did not load within 60s"
