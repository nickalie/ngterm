"""The test-only code of the workspace's Rust sources: what `cargo test` compiles and a non-test build leaves out, kept
out of the unit-test coverage's denominator (`code_coverage.py unit`).

On stable Rust, cargo-llvm-cov instruments `#[cfg(test)]` code with the rest, and the tests that run it count it as
covered, which would credit a crate with its own tests. Test code is found by its attributes, read from the sources by
a lexer that knows Rust's comments (nested block comments too) and literals (strings, raw strings, byte and C strings,
chars and bytes, told from lifetimes), so neither a `//` in a string nor a `#[cfg(test)]` in a comment misleads it:

- an item is test-only when one of its outer attributes is `#[test]` (or a `path::test` such as `tokio::test`), or a
  `#[cfg(...)]` whose predicate can only hold under `test`: `test`, an `all(...)` with such a member, an `any(...)` of
  only such members, a `not` of one that holds whenever `test` is not set. Its span runs from its first attribute
  through its end: the `;` or `,` that ends it, the close of its body, or the close of the group around it;
- a file is test-only when an inner `#![cfg(...)]` of that kind opens it, or when a `mod name;` declares it from
  test-only code: under such an attribute, inside a test-only item, or in a test-only file. The file is found as rustc
  finds it: `#[path]` (relative to the declaring file's directory, or below its inline modules), else `name.rs` or
  `name/mod.rs` beside a `mod.rs`/`lib.rs`/`main.rs`, below the declaring file's own directory otherwise (another crate
  root, such as a `src/bin` program, is tried both ways). A file a non-test declaration also reaches is kept.

A predicate it cannot read (a macro in a cfg) leaves the item counted, and a source it cannot read (an unterminated
literal, an unbalanced group) keeps its inline test code: what it misses stays in the denominator, never production
code out of it.
"""

from __future__ import annotations

import bisect
import re
from collections.abc import Iterable, Sequence
from dataclasses import dataclass, field
from pathlib import Path

# One token: an identifier or keyword (raw identifiers keep their `r#`), a literal, a lifetime, or one punctuation
# character. Whitespace and comments are skipped.
_IDENT = re.compile(r"[^\W\d]\w*")
_RAW_IDENT = re.compile(r"r#[^\W\d]\w*")
_RAW_STRING = re.compile(r'(?:br|cr|r)(#*)"')
_STRING_BODY = re.compile(r'(?:[^"\\]|\\.)*"', re.DOTALL)
_CHAR = re.compile(r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F_]*\}|.)|[^\\'\n])'")
_LIFETIME = re.compile(r"'(?:r#)?[^\W\d]\w*")
_NUMBER = re.compile(r"\d\w*(?:\.\d\w*)?")
_SPACE = re.compile(r"\s+")

IDENT, LITERAL, LIFETIME, PUNCT = "ident", "literal", "lifetime", "punct"
_OPEN = {"(": ")", "[": "]", "{": "}"}
_CLOSE = {")", "]", "}"}

# Items whose body is a `{...}` group (or that end at a `;`); what follows a commas-separated list's member or a
# statement ends at a `,` or `;` too.
_ITEMS = frozenset({"fn", "mod", "impl", "trait", "struct", "union", "enum", "macro_rules"})
# Statements and items that end only at their `;`: a `{...}` inside them (a `use` list, a struct literal) does not.
_STATEMENTS = frozenset({"use", "static", "type", "let"})
# Words that may come before an item's keyword.
_QUALIFIERS = frozenset({"default", "async", "unsafe", "safe", "const", "extern", "auto"})


class LexError(ValueError):
    """The source is not Rust this lexer can read (an unterminated literal or comment, an unbalanced group)."""


@dataclass(frozen=True)
class Token:
    kind: str
    text: str
    line: int
    # the last line of the token (a multi-line string's)
    end: int


def _skip_block_comment(source: str, at: int) -> int:
    """The index past the block comment opening at `at`, nested ones included."""
    depth, i = 0, at
    while True:
        opened, closed = source.find("/*", i), source.find("*/", i)
        if closed < 0:
            raise LexError(f"unterminated block comment at offset {at}")
        if 0 <= opened < closed:
            depth, i = depth + 1, opened + 2
            continue
        depth, i = depth - 1, closed + 2
        if depth == 0:
            return i


def _literal_end(source: str, at: int) -> int | None:
    """The index past a string-like literal starting at `at` (a prefix included), or None when none starts there."""
    raw = _RAW_STRING.match(source, at)
    if raw:
        close = source.find('"' + raw.group(1), raw.end())
        if close < 0:
            raise LexError(f"unterminated raw string at offset {at}")
        return close + 1 + len(raw.group(1))
    start = at + 1 if source.startswith(('b"', 'c"'), at) else at
    if source.startswith('"', start):
        body = _STRING_BODY.match(source, start + 1)
        if not body:
            raise LexError(f"unterminated string at offset {at}")
        return body.end()
    if source.startswith("b'", at):
        char = _CHAR.match(source, at + 1)
        if not char:
            raise LexError(f"unreadable byte literal at offset {at}")
        return char.end()
    return None


def tokens(source: str) -> list[Token]:
    """`source`'s tokens, comments and whitespace left out."""
    out: list[Token] = []
    i, line, size = 0, 1, len(source)
    while i < size:
        char = source[i]
        if char.isspace():
            end = _SPACE.match(source, i).end()  # type: ignore[union-attr]
        elif source.startswith("//", i):
            newline = source.find("\n", i)
            end = size if newline < 0 else newline
        elif source.startswith("/*", i):
            end = _skip_block_comment(source, i)
        else:
            end, kind = _token_end(source, i)
            text = source[i:end]
            out.append(Token(kind, text, line, line + text.count("\n")))
        line += source.count("\n", i, end)
        i = end
    return out


def _token_end(source: str, i: int) -> tuple[int, str]:
    """The end and kind of the token at `i`, which is neither whitespace nor a comment."""
    literal = _literal_end(source, i)
    if literal is not None:
        return literal, LITERAL
    raw_ident = _RAW_IDENT.match(source, i)
    if raw_ident:
        return raw_ident.end(), IDENT
    ident = _IDENT.match(source, i)
    if ident:
        return ident.end(), IDENT
    if source[i] == "'":
        char = _CHAR.match(source, i)
        if char:
            return char.end(), LITERAL
        lifetime = _LIFETIME.match(source, i)
        if lifetime:
            return lifetime.end(), LIFETIME
        raise LexError(f"unreadable quote at offset {i}")
    number = _NUMBER.match(source, i)
    if number:
        return number.end(), LITERAL
    return i + 1, PUNCT


def matching(toks: Sequence[Token]) -> dict[int, int]:
    """Each group's open and close token by the other's index."""
    pairs: dict[int, int] = {}
    stack: list[int] = []
    for index, token in enumerate(toks):
        if token.kind != PUNCT:
            continue
        if token.text in _OPEN:
            stack.append(index)
        elif token.text in _CLOSE:
            if not stack or _OPEN[toks[stack[-1]].text] != token.text:
                raise LexError(f"unbalanced {token.text!r} on line {token.line}")
            opened = stack.pop()
            pairs[opened], pairs[index] = index, opened
    if stack:
        raise LexError(f"unclosed {toks[stack[-1]].text!r} on line {toks[stack[-1]].line}")
    return pairs


# A cfg predicate: a name (`test`, `unix`), a `key = "value"` pair, or a function (`all`, `any`, `not`) of predicates.
@dataclass(frozen=True)
class Predicate:
    name: str
    args: tuple[Predicate, ...] = ()
    call: bool = False


def parse_predicate(toks: Sequence[Token]) -> Predicate | None:
    """The cfg predicate `toks` spell (the inside of `cfg(...)`), or None when they spell none."""
    parsed, rest = _predicate(list(toks), 0)
    return parsed if parsed is not None and rest == len(toks) else None


def _predicate(toks: list[Token], at: int) -> tuple[Predicate | None, int]:
    if at >= len(toks) or toks[at].kind != IDENT:
        return None, at
    name, at = toks[at].text, at + 1
    if at < len(toks) and toks[at].text == "=":
        if at + 1 < len(toks) and toks[at + 1].kind == LITERAL:
            return Predicate(f"{name}={toks[at + 1].text}"), at + 2
        return None, at
    if at < len(toks) and toks[at].text == "(":
        args: list[Predicate] = []
        at += 1
        while at < len(toks) and toks[at].text != ")":
            arg, at = _predicate(toks, at)
            if arg is None:
                return None, at
            args.append(arg)
            if at < len(toks) and toks[at].text == ",":
                at += 1
        if at >= len(toks):
            return None, at
        return Predicate(name, tuple(args), call=True), at + 1
    return Predicate(name), at


def needs_test(predicate: Predicate) -> bool:
    """Whether `predicate` can hold only when `test` is set (it implies `test`)."""
    if not predicate.call:
        return predicate.name == "test"
    if predicate.name == "all":
        return any(needs_test(arg) for arg in predicate.args)
    if predicate.name == "any":
        return bool(predicate.args) and all(needs_test(arg) for arg in predicate.args)
    if predicate.name == "not" and len(predicate.args) == 1:
        return holds_without_test(predicate.args[0])
    return False


def holds_without_test(predicate: Predicate) -> bool:
    """Whether `predicate` holds whenever `test` is not set (`not(test)` implies it)."""
    if not predicate.call:
        return False
    if predicate.name == "all":
        return all(holds_without_test(arg) for arg in predicate.args)
    if predicate.name == "any":
        return any(holds_without_test(arg) for arg in predicate.args)
    if predicate.name == "not" and len(predicate.args) == 1:
        return needs_test(predicate.args[0])
    return False


@dataclass(frozen=True)
class Attribute:
    """One attribute's inside (the tokens between `#[` and `]`) read for what this module needs."""

    test_only: bool
    # `#[path = "..."]`'s value
    path: str | None


def read_attribute(toks: Sequence[Token]) -> Attribute:
    names = [token.text for token in toks]
    if names == ["test"] or (len(names) >= 3 and names[-1] == "test" and names[-2:-1] == [":"]):
        return Attribute(True, None)
    if len(names) >= 3 and names[0] == "cfg" and names[1] == "(" and names[-1] == ")":
        predicate = parse_predicate(toks[2:-1])
        return Attribute(predicate is not None and needs_test(predicate), None)
    if len(names) == 3 and names[0] == "path" and names[1] == "=" and toks[2].kind == LITERAL:
        return Attribute(False, _string_value(toks[2].text))
    return Attribute(False, None)


def _string_value(literal: str) -> str | None:
    raw = _RAW_STRING.match(literal)
    if raw:
        return literal[raw.end() : len(literal) - 1 - len(raw.group(1))]
    if literal.startswith('"') and literal.endswith('"') and "\\" not in literal:
        return literal[1:-1]
    return None


@dataclass(frozen=True)
class ModDecl:
    """A `mod name;`: the file it names is found from these."""

    name: str
    line: int
    # its `#[path]`, if any
    path: str | None
    # the inline modules around it, outermost first: each one's directory (its name, or its own `#[path]`)
    inline: tuple[str, ...]
    # under a test-only attribute of its own
    test_only: bool


@dataclass
class FileScan:
    """One file's test-only spans (first and last line, merged and sorted) and its `mod name;` declarations."""

    spans: list[tuple[int, int]] = field(default_factory=list)
    # an inner `#![cfg(test)]` at the top: the whole file
    whole: bool = False
    decls: list[ModDecl] = field(default_factory=list)

    def covers(self, line: int) -> bool:
        if self.whole:
            return True
        at = bisect.bisect_right(self.spans, (line, float("inf"))) - 1
        return at >= 0 and self.spans[at][0] <= line <= self.spans[at][1]


def _merged(spans: Iterable[tuple[int, int]]) -> list[tuple[int, int]]:
    out: list[tuple[int, int]] = []
    for start, end in sorted(spans):
        if out and start <= out[-1][1] + 1:
            out[-1] = (out[-1][0], max(out[-1][1], end))
        else:
            out.append((start, end))
    return out


class _Scanner:
    """One pass over a file's tokens: attributes, the spans of test-only items, `mod` declarations."""

    def __init__(self, toks: list[Token]) -> None:
        self.toks = toks
        self.pairs = matching(toks)
        self.scan = FileScan()
        self.spans: list[tuple[int, int]] = []
        self.handled: set[int] = set()
        # the inline modules open at the current token: (directory, index of its closing brace)
        self.inline: list[tuple[str, int]] = []
        # the brace groups open at the current token, by their opening token's index
        self.groups: list[int] = []

    def run(self) -> FileScan:
        toks, index = self.toks, 0
        while index < len(toks):
            while self.inline and index > self.inline[-1][1]:
                self.inline.pop()
            while self.groups and index > self.pairs[self.groups[-1]]:
                self.groups.pop()
            token = toks[index]
            if token.text == "#" and token.kind == PUNCT:
                index = self._attributes(index)
                continue
            if token.text == "{" and token.kind == PUNCT:
                self.groups.append(index)
            elif token.kind == IDENT and token.text == "mod" and index not in self.handled:
                self._module(index, [])
            index += 1
        self.scan.spans = _merged(self.spans)
        return self.scan

    def _attribute_at(self, index: int) -> tuple[bool, int] | None:
        """At a `#`: whether the attribute is inner, and the index of its `[`; None when no attribute starts there."""
        toks = self.toks
        inner = index + 1 < len(toks) and toks[index + 1].text == "!"
        bracket = index + 2 if inner else index + 1
        if bracket < len(toks) and toks[bracket].text == "[" and bracket in self.pairs:
            return inner, bracket
        return None

    def _attributes(self, index: int) -> int:
        """Reads the run of attributes at `index`; returns the index past it."""
        found = self._attribute_at(index)
        if found is None:
            return index + 1
        inner, bracket = found
        if inner:
            attribute = read_attribute(self.toks[bracket + 1 : self.pairs[bracket]])
            if attribute.test_only:
                if self.groups:
                    opened = self.groups[-1]
                    self.spans.append((self.toks[opened].line, self.toks[self.pairs[opened]].end))
                else:
                    self.scan.whole = True
            return self.pairs[bracket] + 1
        first, attributes = index, []
        while found is not None and not found[0]:
            bracket = found[1]
            attributes.append(read_attribute(self.toks[bracket + 1 : self.pairs[bracket]]))
            index = self.pairs[bracket] + 1
            found = self._attribute_at(index) if index < len(self.toks) and self.toks[index].text == "#" else None
        test_only = any(attribute.test_only for attribute in attributes)
        if test_only:
            end = self._item_end(index)
            self.spans.append((self.toks[first].line, self.toks[end].end))
        keyword = self._after_visibility(index)
        if keyword < len(self.toks) and self.toks[keyword].text == "mod":
            self._module(keyword, attributes)
        return index

    def _after_visibility(self, index: int) -> int:
        toks = self.toks
        if index < len(toks) and toks[index].text == "pub":
            index += 1
            if index < len(toks) and toks[index].text == "(":
                index = self.pairs[index] + 1
        return index

    def _module(self, index: int, attributes: list[Attribute]) -> None:
        """At a `mod` keyword: records `mod name;`, enters `mod name { ... }`."""
        toks = self.toks
        self.handled.add(index)
        if index + 2 >= len(toks) or toks[index + 1].kind != IDENT:
            return
        name = toks[index + 1].text.removeprefix("r#")
        path = next((attribute.path for attribute in attributes if attribute.path is not None), None)
        after = toks[index + 2]
        if after.text == ";":
            test_only = any(attribute.test_only for attribute in attributes)
            directories = tuple(directory for directory, _close in self.inline)
            self.scan.decls.append(ModDecl(name, toks[index].line, path, directories, test_only))
        elif after.text == "{" and index + 2 in self.pairs:
            self.inline.append((path if path is not None else name, self.pairs[index + 2]))

    def _item_end(self, start: int) -> int:
        """The index of the last token of the item (or field, variant, arm, statement) starting at `start`."""
        toks, pairs = self.toks, self.pairs
        kind = self._kind(start)
        index = start
        while index < len(toks):
            token = toks[index]
            if token.kind == PUNCT:
                if token.text in _OPEN:
                    close = pairs[index]
                    if token.text == "{" and kind != "statement":
                        return close
                    index = close + 1
                    continue
                if token.text in _CLOSE:
                    return max(start, index - 1)
                if token.text == ";" or (token.text == "," and kind == "other"):
                    return index
            index += 1
        return len(toks) - 1

    def _kind(self, start: int) -> str:
        """`item` (ends at its body or `;`), `statement` (ends at its `;`) or `other` (ends at a `,`, `;` or body)."""
        toks = self.toks
        index = self._after_visibility(start)
        while index < len(toks) and toks[index].kind == IDENT:
            word = toks[index].text
            if word in _ITEMS:
                return "item"
            if word in _STATEMENTS:
                return "statement"
            following = toks[index + 1] if index + 1 < len(toks) else None
            if word == "extern" and following is not None:
                if following.text == "crate":
                    return "statement"
                if following.kind == LITERAL:
                    index += 2
                    continue
                if following.text == "{":
                    return "item"
            if word == "const" and following is not None and following.kind == IDENT:
                if following.text not in _ITEMS and following.text not in _QUALIFIERS:
                    return "statement"
            if word == "const" and following is not None and following.text == "_":
                return "statement"
            if word not in _QUALIFIERS:
                return "other"
            index += 1
        return "other"


def scan_source(source: str) -> FileScan:
    """`source`'s test-only spans and `mod` declarations; raises LexError on a source it cannot read."""
    return _Scanner(tokens(source)).run()


def module_file(declaring: Path, decl: ModDecl) -> Path | None:
    """The file `decl` in `declaring` names, as rustc resolves it; None when no such file exists."""
    mod_rs = declaring.name in ("mod.rs", "lib.rs", "main.rs")
    bases = [declaring.parent] if mod_rs else [declaring.parent / declaring.stem, declaring.parent]
    for base in bases:
        below = base.joinpath(*decl.inline)
        if decl.path is not None:
            candidates = [(below if decl.inline else declaring.parent) / decl.path]
        else:
            candidates = [below / f"{decl.name}.rs", below / decl.name / "mod.rs"]
        for candidate in candidates:
            if candidate.is_file():
                return candidate.resolve()
    return None


@dataclass
class TestCode:
    """The test-only code of a set of sources: whole files, and spans of lines in the others."""

    files: set[Path] = field(default_factory=set)
    spans: dict[Path, FileScan] = field(default_factory=dict)
    # sources the lexer could not read, with the reason: their inline test code stays counted
    unread: dict[Path, str] = field(default_factory=dict)

    def whole(self, path: Path) -> bool:
        return path in self.files

    def covers(self, path: Path, line: int) -> bool:
        scan = self.spans.get(path)
        return path in self.files or (scan is not None and scan.covers(line))


def find_test_code(sources: Iterable[Path]) -> TestCode:
    """The test-only code among `sources` (absolute paths of every `.rs` file of the crates to read)."""
    found = TestCode()
    scans: dict[Path, FileScan] = {}
    for path in sorted(set(sources)):
        try:
            scans[path] = scan_source(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, LexError) as error:
            found.unread[path] = str(error)
    targets = {
        (path, decl): module_file(path, decl) for path, scan in scans.items() for decl in scan.decls
    }
    test_files = {path for path, scan in scans.items() if scan.whole}
    changed = True
    while changed:
        changed = False
        for (path, decl), target in targets.items():
            if target is None or target in test_files:
                continue
            if decl.test_only or path in test_files or scans[path].covers(decl.line):
                test_files.add(target)
                changed = True
    live = {
        target
        for (path, decl), target in targets.items()
        if target is not None
        and path not in test_files
        and not decl.test_only
        and not scans[path].covers(decl.line)
    }
    found.files = {path for path in test_files if path not in live or scans.get(path, FileScan()).whole}
    found.spans = {path: scan for path, scan in scans.items() if scan.spans and path not in found.files}
    return found
