from pathlib import Path

import pytest
from agterm_conformance import rust_test_code
from agterm_conformance.rust_test_code import LexError, find_test_code, scan_source, tokens


def spans(source: str) -> list[tuple[int, int]]:
    return scan_source(source).spans


def texts(source: str) -> list[str]:
    return [token.text for token in tokens(source)]


# The lexer: comments and literals are single tokens or nothing, whatever they hold.


def test_comments_are_skipped_nested_block_comments_whole() -> None:
    source = "a // #[cfg(test)] {\n/* x /* #[cfg(test)] */ { */ b /** doc */ c //! inner\nd"
    assert texts(source) == ["a", "b", "c", "d"]
    assert [token.line for token in tokens(source)] == [1, 2, 2, 3]


def test_strings_hold_comment_markers_quotes_and_braces() -> None:
    source = r'''a "// not a comment { \" /* " b'''
    assert texts(source) == ["a", r'"// not a comment { \" /* "', "b"]


@pytest.mark.parametrize(
    "literal",
    [
        'r"{ // }"',
        'r#"a "quoted" { brace"#',
        'r##"a "# still inside {"##',
        'br"bytes {"',
        'cr#"c { "string"#',
        'b"bytes \\" {"',
        'c"c string {"',
    ],
)
def test_raw_byte_and_c_strings_are_one_literal(literal: str) -> None:
    assert texts(f"x {literal} y") == ["x", literal, "y"]


def test_a_multi_line_string_keeps_the_lines_after_it() -> None:
    toks = tokens('a "one\ntwo\nthree" b')
    assert [(token.text, token.line, token.end) for token in toks][1:] == [('"one\ntwo\nthree"', 1, 3), ("b", 3, 3)]


@pytest.mark.parametrize("char", ["'{'", "'}'", "'\"'", r"'\''", r"'\\'", r"'\u{1F600}'", r"'\x7f'", "'é'", "b'{'",
                                  r"b'\''"])
def test_char_and_byte_literals_are_one_literal(char: str) -> None:
    toks = tokens(f"match c {{ {char} => 1 }}")
    assert [token.text for token in toks] == ["match", "c", "{", char, "=", ">", "1", "}"]
    assert toks[3].kind == rust_test_code.LITERAL


def test_lifetimes_and_labels_are_not_chars() -> None:
    toks = tokens("fn f<'a,'b>(x: &'a str) -> &'static str { 'outer: loop { break 'outer; } }")
    lifetimes = [token.text for token in toks if token.kind == rust_test_code.LIFETIME]
    assert lifetimes == ["'a", "'b", "'a", "'static", "'outer", "'outer"]
    assert rust_test_code.matching(toks)  # every brace still pairs


def test_raw_identifiers_are_identifiers_not_raw_strings() -> None:
    assert texts('r#type r#"s"#') == ["r#type", 'r#"s"#']


@pytest.mark.parametrize("source", ['"open', "/* open", 'r#"open"', "fn f( {", "}"])
def test_unreadable_sources_raise(source: str) -> None:
    with pytest.raises(LexError):
        scan_source(source)


# Which attributes make an item test-only.


@pytest.mark.parametrize(
    ("attribute", "test_only"),
    [
        ("#[cfg(test)]", True),
        ("#[cfg(all(test, unix))]", True),
        ('#[cfg(all(target_os = "linux", any(test, test)))]', True),
        ("#[cfg(not(not(test)))]", True),
        ("#[cfg(not(any(not(test), unix)))]", True),
        ("#[test]", True),
        ("#[tokio::test]", True),
        ('#[cfg(any(test, feature = "fakes"))]', False),
        ("#[cfg(not(test))]", False),
        ("#[cfg(unix)]", False),
        ('#[cfg(feature = "test")]', False),
        ("#[cfg_attr(test, derive(Debug))]", False),
        ("#[cfg(all())]", False),
        ("#[cfg(any())]", False),
        ("#[cfg(not(all(unix, not(test))))]", False),
        ("#[should_panic]", False),
    ],
)
def test_a_cfg_that_can_only_hold_under_test_makes_the_item_test_only(attribute: str, test_only: bool) -> None:
    assert spans(f"fn a() {{}}\n{attribute}\nfn b() {{\n}}\nfn c() {{}}\n") == ([(2, 4)] if test_only else [])


def test_any_test_only_attribute_of_a_run_makes_the_item_test_only() -> None:
    source = (
        '#[allow(dead_code)]\n#[cfg(unix)]\n#[cfg(test)]\n/// doc\n#[path = "x.rs"]\nmod helper {\n}\nfn after() {}\n'
    )
    assert spans(source) == [(1, 7)]


# Where a test-only item ends.


def test_a_test_module_spans_from_its_attribute_to_its_closing_brace() -> None:
    source = """fn production() {}

#[cfg(test)]
mod tests {
    use super::*;
    const BRACE: char = '}';
    const TEXT: &str = "}";

    #[test]
    fn works() {
        assert_eq!(production(), ());
    }
}
"""
    assert spans(source) == [(3, 13)]


@pytest.mark.parametrize(
    ("item", "lines"),
    [
        ("#[cfg(test)]\nfn f<A, B>(a: A, b: B) -> [u8; 2]\nwhere\n    A: Copy,\n{\n}\n", (1, 6)),
        ("#[cfg(test)]\nstruct Unit;\n", (1, 2)),
        ("#[cfg(test)]\nstruct Pair<A, B>(A, B);\n", (1, 2)),
        ("#[cfg(test)]\nuse std::{\n    io,\n    fs,\n};\n", (1, 5)),
        ("#[cfg(test)]\nconst X: Point = Point {\n    x: 1,\n};\n", (1, 4)),
        ("#[cfg(test)]\nconst fn f() -> u8 {\n    1\n}\n", (1, 4)),
        ("#[cfg(test)]\nstatic S: [u8; 2] = [1, 2];\n", (1, 2)),
        ("#[cfg(test)]\nimpl<A, B> Trait for Pair<A, B> {\n}\n", (1, 3)),
        ("#[cfg(test)]\npub(crate) unsafe extern \"C\" fn f() {\n}\n", (1, 3)),
        ("#[cfg(test)]\nmacro_rules! m {\n    () => {};\n}\n", (1, 4)),
        ("#[cfg(test)]\nthread_local! {\n    static X: u8 = 1;\n}\n", (1, 4)),
        ("#[cfg(test)]\nextern crate alloc;\n", (1, 2)),
    ],
)
def test_an_item_ends_at_its_body_or_its_semicolon(item: str, lines: tuple[int, int]) -> None:
    assert spans(item + "fn after() {\n}\n") == [lines]


def test_a_field_variant_or_statement_ends_at_its_comma_semicolon_or_group() -> None:
    source = """struct S {
    a: u8,
    #[cfg(test)]
    b: u8,
    c: u8,
}
enum E {
    #[cfg(test)]
    V { x: u8 },
    W,
    #[cfg(test)]
    Last
}
fn f() {
    let a = 1;
    #[cfg(test)]
    let b = vec![1, 2];
    #[cfg(test)]
    {
        check(a);
    }
    done(a);
}
"""
    # the let and the block are adjacent: one span
    assert spans(source) == [(3, 4), (8, 9), (11, 12), (16, 21)]


def test_an_inner_cfg_test_makes_its_file_or_its_module_test_only() -> None:
    assert scan_source("//! docs\n#![cfg(test)]\nfn f() {}\n").whole
    source = "fn a() {}\nmod helpers {\n    #![cfg(test)]\n    fn h() {}\n}\nfn b() {}\n"
    scan = scan_source(source)
    assert not scan.whole
    assert scan.spans == [(2, 5)]
    assert not scan_source("#![allow(dead_code)]\nfn f() {}\n").whole


def test_the_markers_in_comments_and_strings_make_nothing_test_only() -> None:
    source = '// #[cfg(test)]\nfn a() { let s = "#[cfg(test)] mod x;"; }\n/* #[cfg(test)]\nmod tests; */\nfn b() {}\n'
    scan = scan_source(source)
    assert scan.spans == [] and scan.decls == []


def test_covers_answers_by_line() -> None:
    scan = scan_source("fn a() {}\n#[cfg(test)]\nfn b() {\n}\n#[test]\nfn c() {}\n")
    assert [line for line in range(1, 8) if scan.covers(line)] == [2, 3, 4, 5, 6]


# Module files.


def write(root: Path, files: dict[str, str]) -> list[Path]:
    for name, text in files.items():
        (root / name).parent.mkdir(parents=True, exist_ok=True)
        (root / name).write_text(text)
    return [(root / name).resolve() for name in files]


def test_test_only_module_files_are_found_by_their_declarations(tmp_path: Path) -> None:
    sources = write(
        tmp_path,
        {
            "src/lib.rs": "#[cfg(test)]\nmod test_shell;\nmod model;\npub mod nested;\n#[cfg(test)]\nmod helpers;\n",
            "src/test_shell.rs": "pub fn shell() {}\n",
            "src/model.rs": '#[cfg(test)]\n#[path = "model_support.rs"]\nmod support;\nfn m() {}\n',
            "src/model_support.rs": "mod deeper;\n",
            "src/model_support/deeper.rs": "fn d() {}\n",
            "src/nested/mod.rs": (
                "#[cfg(all(test, unix))]\nmod testing;\nmod inline {\n    #[cfg(test)]\n    mod t;\n}\n"
            ),
            "src/nested/testing.rs": "fn t() {}\n",
            "src/nested/inline/t.rs": "fn t() {}\n",
            "src/helpers/mod.rs": "mod more;\n",
            "src/helpers/more.rs": "fn more() {}\n",
        },
    )
    found = find_test_code(sources)
    relative = sorted(path.relative_to(tmp_path.resolve()).as_posix() for path in found.files)
    # model_support.rs: a #[path] beside its declaring file; deeper.rs: what a test-only file declares
    assert relative == [
        "src/helpers/mod.rs",
        "src/helpers/more.rs",
        "src/model_support.rs",
        "src/model_support/deeper.rs",
        "src/nested/inline/t.rs",
        "src/nested/testing.rs",
        "src/test_shell.rs",
    ]
    lib = (tmp_path / "src/lib.rs").resolve()
    assert found.spans[lib].spans == [(1, 2), (5, 6)]
    assert found.covers(lib, 1) and not found.covers(lib, 3)
    assert found.whole((tmp_path / "src/test_shell.rs").resolve())


def test_a_module_inside_a_test_module_is_test_only(tmp_path: Path) -> None:
    sources = write(
        tmp_path,
        {
            "src/main.rs": "#[cfg(test)]\nmod tests {\n    mod support;\n}\nmod live;\n",
            "src/tests/support.rs": "fn s() {}\n",
            "src/live.rs": "fn l() {}\n",
        },
    )
    found = find_test_code(sources)
    assert found.files == {(tmp_path / "src/tests/support.rs").resolve()}


def test_a_file_a_production_declaration_also_reaches_is_kept(tmp_path: Path) -> None:
    sources = write(
        tmp_path,
        {
            "src/lib.rs": '#[cfg(test)]\n#[path = "shared.rs"]\nmod shared_for_tests;\nmod shared;\n',
            "src/shared.rs": "fn s() {}\n",
        },
    )
    assert find_test_code(sources).files == set()


def test_a_program_root_finds_its_modules_beside_it(tmp_path: Path) -> None:
    sources = write(tmp_path, {"src/bin/tool.rs": "#[cfg(test)]\nmod tool_tests;\n", "src/bin/tool_tests.rs": ""})
    assert find_test_code(sources).files == {(tmp_path / "src/bin/tool_tests.rs").resolve()}


def test_an_unreadable_source_is_named_and_left_counted(tmp_path: Path) -> None:
    sources = write(tmp_path, {"src/lib.rs": "#[cfg(test)]\nmod tests;\n", "src/tests.rs": 'fn f() { "open }\n'})
    found = find_test_code(sources)
    broken = (tmp_path / "src/tests.rs").resolve()
    assert broken in found.unread
    assert found.files == {broken}  # declared from test code, so still out
