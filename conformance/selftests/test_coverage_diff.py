import importlib.util
import shutil
import sys
from pathlib import Path
from types import ModuleType

import pytest

TOOLS = Path(__file__).resolve().parent.parent / "tools"


def _load(name: str) -> ModuleType:
    spec = importlib.util.spec_from_file_location(name, TOOLS / f"{name}.py")
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module  # dataclasses resolve their module by name
    spec.loader.exec_module(module)
    return module


cd = _load("coverage_diff")

# A legacy-mangled Rust symbol: ngterm_gpui::state::State::apply
APPLY = "_ZN11ngterm_gpui5state5State5apply17h0123456789abcdefE"
# a closure in it: ngterm_gpui::state::State::apply::{{closure}}
APPLY_CLOSURE = "_ZN11ngterm_gpui5state5State5apply28_$u7b$$u7b$closure$u7d$$u7d$17h0123456789abcdefE"


def lcov(files: dict[str, dict[int, int]], functions: dict[str, list[tuple[int, str]]] | None = None) -> str:
    """An lcov.info as code_coverage.render_lcov writes it: per file its functions and DA lines."""
    out: list[str] = []
    for name, lines in files.items():
        out.append(f"SF:{name}")
        for start, function in (functions or {}).get(name, []):
            out += [f"FN:{start},{function}", f"FNDA:1,{function}"]
        out += [f"DA:{number},{count}" for number, count in sorted(lines.items())]
        out += [f"LF:{len(lines)}", f"LH:{sum(1 for count in lines.values() if count)}", "end_of_record"]
    return "".join(f"{line}\n" for line in out)


def write(directory: Path, name: str, text: str) -> Path:
    path = directory / name
    path.write_text(text, encoding="utf-8")
    return path


def run(capsys: pytest.CaptureFixture[str], *argv: str | Path) -> tuple[int, str, str]:
    code = cd.main([str(arg) for arg in argv])
    out, err = capsys.readouterr()
    return code, out, err


GPUI = "/__w/ngterm/ngterm/crates/ngterm-gpui/src/state.rs"
CORE = "/__w/ngterm/ngterm/crates/ngterm-core/src/model.rs"


def test_identical_runs_report_no_varying_line(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    text = lcov({GPUI: {1: 1, 2: 0, 3: 5}, CORE: {10: 2}})
    a, b = write(tmp_path, "a.info", text), write(tmp_path, "b.info", text)
    code, out, _ = run(capsys, a, b, "--max-spread", "0.1")
    assert code == 0
    assert "no line varies" in out
    assert "| ngterm-gpui | 66.67% (2/3) | 66.67% (2/3) | 0.00 |" in out


def test_a_line_hit_in_one_run_only_is_named_with_its_runs_and_function(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    functions = {GPUI: [(1, APPLY)]}
    a = write(tmp_path, "a.info", lcov({GPUI: {1: 1, 2: 3, 3: 4, 4: 1}}, functions))
    b = write(tmp_path, "b.info", lcov({GPUI: {1: 1, 2: 0, 3: 0, 4: 1}}, functions))
    c = write(tmp_path, "c.info", lcov({GPUI: {1: 1, 2: 2, 3: 7, 4: 1}}, functions))
    code, out, _ = run(capsys, a, b, c)
    assert code == 0
    # lines 2-3 vary together: one group, hit by runs 1 and 3, not 2, inside State::apply
    assert "`crates/ngterm-gpui/src/state.rs:2-3`" in out
    assert "2 lines" in out
    assert "hit by 1, 3; missed by 2" in out
    assert "ngterm_gpui::state::State::apply" in out
    assert out.count("crates/ngterm-gpui/src/state.rs:") == 1


def test_a_line_below_a_closure_names_the_closure_and_its_enclosing_function(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    # lcov gives functions their start only: line 6 is in the closure starting on 3 or after it, in apply
    functions = {GPUI: [(1, APPLY), (3, APPLY_CLOSURE)]}
    a = write(tmp_path, "a.info", lcov({GPUI: {1: 1, 3: 1, 6: 1}}, functions))
    b = write(tmp_path, "b.info", lcov({GPUI: {1: 1, 3: 1, 6: 0}}, functions))
    _, out, _ = run(capsys, a, b)
    assert (
        "in `ngterm_gpui::state::State::apply::{{closure}}`, or after it in `ngterm_gpui::state::State::apply`" in out
    )


def test_adjacent_lines_with_another_pattern_split_the_group(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    a = write(tmp_path, "a.info", lcov({GPUI: {5: 1, 6: 0, 7: 1, 9: 1}}))
    b = write(tmp_path, "b.info", lcov({GPUI: {5: 0, 6: 1, 7: 1, 9: 0}}))
    _, out, _ = run(capsys, a, b)
    assert "state.rs:5`" in out
    assert "state.rs:6`" in out
    # line 7 runs in both: 9 is its own group, not joined to 5
    assert "state.rs:9`" in out
    assert "state.rs:5-9" not in out


def test_runs_on_other_roots_and_separators_compare_by_repo_path(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    a = write(tmp_path, "a.info", lcov({GPUI: {1: 1, 2: 1}}))
    b = write(tmp_path, "b.info", lcov({r"D:\a\ngterm\ngterm\crates\ngterm-gpui\src\state.rs": {1: 1, 2: 0}}))
    _, out, _ = run(capsys, a, b)
    assert "`crates/ngterm-gpui/src/state.rs:2`" in out
    assert "measured in some runs only" not in out


def test_a_spread_over_the_limit_fails_and_names_the_crate(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    lines = {number: 1 for number in range(1, 101)}
    missed = {**lines, 100: 0}
    a = write(tmp_path, "a.info", lcov({GPUI: lines, CORE: {1: 1}}))
    b = write(tmp_path, "b.info", lcov({GPUI: missed, CORE: {1: 1}}))
    code, out, err = run(capsys, a, b, "--max-spread", "0.5")
    assert code == 1
    assert "| ngterm-gpui | 100.00% (100/100) | 99.00% (99/100) | 1.00 |" in out
    assert "ngterm-gpui" in err and "1.00 pt" in err
    assert "ngterm-core" not in err
    code, _, _ = run(capsys, a, b, "--max-spread", "1")
    assert code == 0


def test_a_line_only_some_runs_measure_is_listed_apart(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    a = write(tmp_path, "a.info", lcov({GPUI: {1: 1, 2: 0}}))
    b = write(tmp_path, "b.info", lcov({GPUI: {1: 1}}))
    _, out, _ = run(capsys, a, b)
    assert "measured in some runs only" in out
    assert "`crates/ngterm-gpui/src/state.rs:2`" in out


def test_labels_name_the_runs(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    a = write(tmp_path, "a.info", lcov({GPUI: {1: 1}}))
    b = write(tmp_path, "b.info", lcov({GPUI: {1: 0}}))
    _, out, _ = run(capsys, a, b, "--label", "first", "--label", "second")
    assert "| crate | first | second | spread |" in out
    assert "hit by first; missed by second" in out


def test_the_source_line_is_quoted_from_the_repo(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    source = tmp_path / "repo" / "crates" / "ngterm-gpui" / "src" / "state.rs"
    source.parent.mkdir(parents=True)
    source.write_text("fn apply() {\n    self.redraw();\n}\n", encoding="utf-8")
    a = write(tmp_path, "a.info", lcov({GPUI: {2: 1}}))
    b = write(tmp_path, "b.info", lcov({GPUI: {2: 0}}))
    _, out, _ = run(capsys, a, b, "--repo", tmp_path / "repo")
    assert "self.redraw();" in out


def test_a_source_outside_the_repo_is_not_quoted(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    outside = tmp_path / "outside" / "local.rs"
    outside.parent.mkdir()
    outside.write_text("let secret = 1;\n", encoding="utf-8")
    (tmp_path / "repo").mkdir()
    for name in (str(outside), "../outside/local.rs"):
        a = write(tmp_path, "a.info", lcov({name: {1: 1}}))
        b = write(tmp_path, "b.info", lcov({name: {1: 0}}))
        _, out, _ = run(capsys, a, b, "--repo", tmp_path / "repo")
        assert "local.rs:1`" in out
        assert "secret" not in out


def test_test_names_blank_lines_and_a_bom_are_lcov(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    a = write(tmp_path, "a.info", "\ufeffTN:\n\n" + lcov({GPUI: {1: 1}}))
    b = write(tmp_path, "b.info", "TN:suite\n" + lcov({GPUI: {1: 0}}) + "\n")
    code, out, _ = run(capsys, a, b)
    assert code == 0
    assert "state.rs:1`" in out


def test_the_report_goes_to_out(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    a = write(tmp_path, "a.info", lcov({GPUI: {1: 1}}))
    b = write(tmp_path, "b.info", lcov({GPUI: {1: 0}}))
    code, out, _ = run(capsys, a, b, "--out", tmp_path / "diff.md")
    assert code == 0
    assert "state.rs:1" in (tmp_path / "diff.md").read_text(encoding="utf-8")
    assert "state.rs:1" not in out


@pytest.mark.parametrize(
    ("argv", "message"),
    [
        (["a.info"], "at least 2"),
        (["a.info", "missing.info"], "missing.info"),
        (["a.info", "bad.info"], "bad.info:2"),
        (["a.info", "a.info", "--label", "one"], "--label"),
        # no data is bad input, not a run that hit nothing: the gate must not pass on it
        (["empty.info", "empty.info", "--max-spread", "0.1"], "empty.info: no SF record"),
        (["a.info", "empty.info"], "empty.info: no SF record"),
        (["a.info", "summary.info"], "summary.info:1"),
        (["a.info", "truncated.info"], "truncated.info: the record of crates/ngterm-gpui/src/state.rs"),
        (["a.info", "unclosed.info"], "unclosed.info:3"),
        (["a.info", "junk.info"], "junk.info:3"),
    ],
)
def test_bad_input_exits_2(tmp_path: Path, capsys: pytest.CaptureFixture[str], argv: list[str], message: str) -> None:
    write(tmp_path, "a.info", lcov({GPUI: {1: 1}}))
    write(tmp_path, "bad.info", f"SF:{GPUI}\nDA:one,1\nend_of_record\n")
    write(tmp_path, "empty.info", "")
    write(tmp_path, "summary.info", '{"data": [], "type": "llvm.coverage.json.export"}\n')
    write(tmp_path, "truncated.info", lcov({GPUI: {1: 1, 2: 1}}).removesuffix("end_of_record\n"))
    write(tmp_path, "unclosed.info", f"SF:{GPUI}\nDA:1,1\nSF:{CORE}\nDA:1,1\nend_of_record\n")
    write(tmp_path, "junk.info", f"SF:{GPUI}\nDA:1,1\nnot lcov at all\nend_of_record\n")
    code, _, err = run(capsys, *(tmp_path / arg if arg.endswith(".info") else arg for arg in argv))
    assert code == 2
    assert message in err


@pytest.mark.parametrize(
    ("symbol", "name"),
    [
        (APPLY, "ngterm_gpui::state::State::apply"),
        ("_ZN4core3ptr46drop_in_place$LT$ngterm_core..model..Model$GT$17h0123456789abcdefE",
         "core::ptr::drop_in_place<ngterm_core::model::Model>"),
        ("_RNvCs1234_4core3foo", "_RNvCs1234_4core3foo"),
        ("plain", "plain"),
        # an escape no code point has stays mangled, not a crash
        ("_ZN9$u110000$E", "_ZN9$u110000$E"),
        ("_ZN7$ud800$E", "_ZN7$ud800$E"),
    ],
)
def test_legacy_rust_symbols_are_demangled(symbol: str, name: str) -> None:
    assert cd.demangle(symbol) == name


@pytest.mark.parametrize("tool", ["c++filt", "llvm-cxxfilt"])
def test_v0_symbols_are_demangled_by_cxxfilt_without_disambiguators(
    tool: str, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = shutil.which(tool)
    if not path:
        pytest.skip(f"no {tool} on PATH")
    # the tool demangle_all finds is this one, whichever comes first on PATH
    monkeypatch.setattr(cd.shutil, "which", lambda name: path if name == tool else None)
    symbol = "_RNvNtCscNSrzPEvWDO_10ngterm_app8coverage13write_profile"
    generic = "_ZN4core3ptr46drop_in_place$LT$ngterm_core..model..Model$GT$17h0123456789abcdefE"
    # LLVM's demangler leaves legacy escapes as they are: the built-in one keeps those
    assert cd.demangle_all([symbol, APPLY, generic, APPLY_CLOSURE]) == {
        symbol: "ngterm_app::coverage::write_profile",
        APPLY: "ngterm_gpui::state::State::apply",
        generic: "core::ptr::drop_in_place<ngterm_core::model::Model>",
        APPLY_CLOSURE: "ngterm_gpui::state::State::apply::{{closure}}",
    }
