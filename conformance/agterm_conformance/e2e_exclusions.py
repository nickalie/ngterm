"""The exclusion markers of the e2e coverage report (`code_coverage.py e2e`): code that truly cannot run end to end,
marked in its source next to the code, with the reason.

One comment form in Rust, Kotlin and Swift, on a line of its own:

- `// e2e-coverage: off: <reason>` on the line directly above a function excludes that function: from the line below
  the marker through the end of the function's body. Attributes, annotations, modifiers and comments may sit between
  the marker and the function's keyword, a blank line or other code may not;
- `// e2e-coverage: off-file: <reason>` as the file's first non-blank line excludes the whole file.

A function is what a coverage map counts as one: Rust `fn`; Kotlin `fun`, `constructor` and `init` blocks; Swift
`func`, `init`, `deinit`, `subscript` and a computed `var`. Its body is its `{...}` group, or a Kotlin `= expression`
that runs to the last line the expression continues on (a line ending in an operator, or followed by one starting with
`.`, `?.`, `?:`, `&&`, `||`, `as` or `else`, continues it).

A marker with no reason, of an unknown kind, sharing its line with code, an `off-file` one below the file's first
line, or a function marker with no function directly below it (or one with no body: a trait, interface or protocol
requirement) is refused: `find_markers` raises MarkerError naming each such marker's file and line. A marker inside a
string, a block comment or a doc comment (`///`, `//!`) is text, not a marker: the sources are read by a lexer that
knows each language's comments (nested block comments too) and literals (Rust's raw strings, Kotlin's `${...}`
templates and raw strings, Swift's `\\(...)` interpolations and `#"..."#` raw strings). Rust's is rust_test_code.py's.
"""

from __future__ import annotations

import bisect
import re
import sys
from collections.abc import Sequence
from dataclasses import dataclass

from agterm_conformance.rust_test_code import (
    _IDENT,
    _NUMBER,
    _SPACE,
    COMMENT,
    IDENT,
    LITERAL,
    PUNCT,
    LexError,
    Token,
    _skip_block_comment,
    matching,
    tokens,
)

MARKER_TEXT = "e2e-coverage:"
FUNCTION, FILE = "function", "file"
LANGUAGES = {".rs": "rust", ".kt": "kotlin", ".kts": "kotlin", ".swift": "swift"}

# The words that may come before a function's keyword, and the keywords of what a coverage map counts as a function.
_MODIFIERS = {
    "rust": frozenset({"pub", "const", "async", "unsafe", "safe", "extern", "default"}),
    "kotlin": frozenset({
        "public", "private", "protected", "internal", "override", "open", "final", "abstract", "suspend", "inline",
        "tailrec", "operator", "infix", "external", "actual", "expect",
    }),
    "swift": frozenset({
        "public", "private", "fileprivate", "internal", "package", "open", "final", "static", "class", "override",
        "mutating", "nonmutating", "convenience", "required", "dynamic", "nonisolated", "isolated", "optional",
        "prefix", "postfix", "infix", "distributed", "consuming", "borrowing", "lazy",
    }),
}
_KEYWORDS = {
    "rust": frozenset({"fn"}),
    "kotlin": frozenset({"fun", "constructor", "init"}),
    "swift": frozenset({"func", "init", "deinit", "subscript", "var"}),
}
# Words that start the next declaration: met before a body, the function has none.
_DECLARATIONS = {
    "rust": frozenset(),
    "kotlin": frozenset({"fun", "val", "var", "class", "interface", "object", "typealias", "constructor", "init"}),
    "swift": frozenset({
        "func", "var", "let", "init", "deinit", "subscript", "class", "struct", "enum", "protocol", "extension",
        "typealias", "case", "associatedtype",
    }),
}
# Kotlin: a line ending in one of these, or a next line starting with one of those, continues an expression body.
_ENDS_CONTINUING = frozenset({"=", ",", ".", "(", "[", "{", "+", "-", "*", "/", "%", "<", "&", "|", ":"})
_ENDS_CONTINUING_WORDS = frozenset({"as", "is", "in", "else", "to", "and", "or"})
_STARTS_CONTINUING = frozenset({".", "?", "&", "|"})
_STARTS_CONTINUING_WORDS = frozenset({"as", "else", "catch", "finally"})
# An `=` with one of these before it is part of a comparison or a compound assignment, not an assignment.
_NOT_ASSIGNMENT = frozenset({"=", "!", "<", ">", "+", "-", "*", "/", "%", "&", "|", "^"})

_MARKER = re.compile(r"(off|off-file):(.*)", re.DOTALL)
_FORMS = "`// e2e-coverage: off: <reason>` or `// e2e-coverage: off-file: <reason>`"
_KOTLIN_CHAR = re.compile(r"'(?:\\u[0-9a-fA-F]{4}|\\.|[^\\'\n])'")
_SWIFT_STRING = re.compile(r'(#*)("""|")')
_BACKTICK = re.compile(r"`[^`\n]+`")


class MarkerError(Exception):
    """Markers the report refuses; the message has one `path:line: why` line per marker."""


@dataclass(frozen=True)
class Marker:
    path: str
    # the marker comment's line
    line: int
    scope: str
    reason: str
    # the function's name, empty for a file
    name: str
    # the lines it excludes, first and last
    first: int
    last: int

    def covers(self, line: int) -> bool:
        return self.first <= line <= self.last


def language_of(path: str) -> str | None:
    """The language whose markers `path` holds, by its suffix; None for a file that holds none."""
    dot = path.rfind(".")
    return LANGUAGES.get(path[dot:]) if dot > path.rfind("/") else None


def lex(source: str, language: str) -> list[Token]:
    """`source`'s tokens, comments included; raises LexError on a source the lexer cannot read."""
    if language == "rust":
        return tokens(source, comments=True)
    out: list[Token] = []
    _Lexer(source, language).run(0, None, out)
    return out


class _Lexer:
    """Kotlin's and Swift's tokens: identifiers, literals (a string with its templates or interpolations is one),
    comments and one-character punctuation."""

    def __init__(self, source: str, language: str) -> None:
        self.source, self.language = source, language
        self.newlines = [index for index, char in enumerate(source) if char == "\n"]

    def line(self, at: int) -> int:
        return bisect.bisect_left(self.newlines, at) + 1

    def run(self, at: int, closer: str | None, out: list[Token] | None) -> int:
        """Reads tokens from `at` into `out` (None: skipped); with `closer`, up to the `closer` that ends the template or
        interpolation it is in, returning the index past it."""
        source, depth = self.source, 0
        while at < len(source):
            char = source[at]
            if char.isspace():
                at = _SPACE.match(source, at).end()  # type: ignore[union-attr]
                continue
            if closer is not None and depth == 0 and char == closer:
                return at + 1
            kind, end = self._token(at)
            if kind == PUNCT:
                depth += 1 if char in "([{" else -1 if char in ")]}" else 0
            if out is not None:
                text = source[at:end]
                line = self.line(at)
                out.append(Token(kind, text, line, line + text.count("\n")))
            at = end
        if closer is not None:
            raise LexError(f"unterminated {'template' if closer == '}' else 'interpolation'}")
        return at

    def _token(self, at: int) -> tuple[str, int]:
        source = self.source
        if source.startswith("//", at):
            newline = source.find("\n", at)
            return COMMENT, len(source) if newline < 0 else newline
        if source.startswith("/*", at):
            return COMMENT, _skip_block_comment(source, at)
        string = self._kotlin_string(at) if self.language == "kotlin" else self._swift_string(at)
        if string is not None:
            return LITERAL, string
        for pattern, kind in ((_BACKTICK, IDENT), (_IDENT, IDENT), (_NUMBER, LITERAL)):
            found = pattern.match(source, at)
            if found:
                return kind, found.end()
        return PUNCT, at + 1

    def _kotlin_string(self, at: int) -> int | None:
        source = self.source
        if source.startswith("'", at):
            char = _KOTLIN_CHAR.match(source, at)
            if not char:
                raise LexError(f"unreadable char literal on line {self.line(at)}")
            return char.end()
        if not source.startswith('"', at):
            return None
        raw = source.startswith('"""', at)
        index = at + (3 if raw else 1)
        while index < len(source):
            if raw and source.startswith('"""', index):
                index += 3
                while index < len(source) and source[index] == '"':
                    index += 1
                return index
            if not raw and source[index] == '"':
                return index + 1
            if source.startswith("${", index):
                index = self.run(index + 2, "}", None)
            elif not raw and source[index] == "\\":
                index += 2
            elif not raw and source[index] == "\n":
                break
            else:
                index += 1
        raise LexError(f"unterminated string on line {self.line(at)}")

    def _swift_string(self, at: int) -> int | None:
        source = self.source
        opening = _SWIFT_STRING.match(source, at)
        if not opening:
            return None
        hashes, quote = opening.groups()
        closing, escape = quote + hashes, "\\" + hashes
        index = opening.end()
        while index < len(source):
            if source.startswith(closing, index):
                return index + len(closing)
            if source.startswith(escape + "(", index):
                index = self.run(index + len(escape) + 1, ")", None)
            elif source.startswith(escape, index):
                index += len(escape) + 1
            elif quote == '"' and source[index] == "\n":
                break
            else:
                index += 1
        raise LexError(f"unterminated string on line {self.line(at)}")


def find_markers(source: str, language: str, path: str) -> list[Marker]:
    """The markers in `source` (the file at `path`, named in errors), each resolved to what it excludes; raises
    MarkerError naming every marker it refuses."""
    if MARKER_TEXT not in source:
        return []
    try:
        toks = lex(source, language)
        pairs = matching(toks)
    except LexError as error:
        raise MarkerError(f"{path}: cannot read the source to find its markers: {error}") from None
    found: list[Marker] = []
    errors: list[str] = []
    last_code_line = 0
    for index, token in enumerate(toks):
        if token.kind != COMMENT:
            last_code_line = token.end
            continue
        parsed = _parse(token.text)
        if parsed is None:
            continue
        where = f"{path}:{token.line}"
        if isinstance(parsed, str):
            errors.append(f"{where}: {parsed}")
            continue
        scope, reason = parsed
        if last_code_line == token.line:
            errors.append(f"{where}: a marker shares its line with code (put it on its own line above the function)")
        elif scope == FILE:
            if index != 0:
                errors.append(f"{where}: an off-file marker must be the file's first non-blank line")
            else:
                found.append(Marker(path, token.line, FILE, reason, "", 1, sys.maxsize))
        else:
            resolved = _Resolver(toks, pairs, language).function_below(index)
            if isinstance(resolved, str):
                errors.append(f"{where}: {resolved}")
            else:
                name, last = resolved
                found.append(Marker(path, token.line, FUNCTION, reason, name, token.line + 1, last))
    if errors:
        raise MarkerError("\n".join(errors))
    return found


def _parse(comment: str) -> tuple[str, str] | str | None:
    """A comment's marker: (scope, reason), the reason it is refused, or None when the comment is not a marker."""
    if not comment.startswith("//") or comment.startswith(("///", "//!")):
        return None
    body = comment[2:].strip()
    if not body.startswith(MARKER_TEXT):
        return None
    form = _MARKER.fullmatch(body[len(MARKER_TEXT) :].strip())
    if form is None:
        return f"not {_FORMS}"
    reason = form.group(2).strip()
    if not reason:
        return "a marker with no reason (say why no e2e test can run this code)"
    return (FILE if form.group(1) == "off-file" else FUNCTION), reason


class _Resolver:
    """Finds the function directly below a marker and the last line of its body."""

    def __init__(self, toks: Sequence[Token], pairs: dict[int, int], language: str) -> None:
        self.toks, self.pairs, self.language = toks, pairs, language

    def function_below(self, marker: int) -> tuple[str, int] | str:
        """The function below the marker at `marker`: its name and its body's last line, or why there is none."""
        toks = self.toks
        none = "no function directly below the marker (only attributes, modifiers and comments may sit between)"
        index, previous = marker + 1, toks[marker].end
        while index < len(toks):
            token = toks[index]
            if token.line > previous + 1:
                return none
            if token.kind == COMMENT:
                index, previous = index + 1, token.end
                continue
            after = self._attribute_end(index)
            if after is None:
                after = self._modifier_end(index)
            if after is None:
                break
            previous, index = toks[after - 1].end, after
        if index >= len(toks) or toks[index].kind != IDENT or toks[index].text not in _KEYWORDS[self.language]:
            return none
        last = self._body_end(index)
        if last is None:
            return "the function below has no body to run (a trait, interface or protocol requirement)"
        return self._name(index), toks[last].end

    def _attribute_end(self, index: int) -> int | None:
        """The index past the attribute or annotation at `index`, or None when none starts there."""
        toks, pairs = self.toks, self.pairs
        text = toks[index].text
        if self.language == "rust":
            if text == "#" and index + 1 < len(toks) and toks[index + 1].text == "[":
                return pairs[index + 1] + 1
            return None
        if text != "@" or index + 1 >= len(toks):
            return None
        if toks[index + 1].text == "[":  # Kotlin's `@[A B]`
            return pairs[index + 1] + 1
        at = index + 1
        if toks[at].kind != IDENT:
            return None
        at += 1
        while at + 1 < len(toks) and toks[at].text in (".", ":") and toks[at + 1].kind == IDENT:
            at += 2
        if at < len(toks) and toks[at].text == "(" and toks[at].line == toks[at - 1].end:
            at = pairs[at] + 1
        return at

    def _modifier_end(self, index: int) -> int | None:
        toks = self.toks
        token = toks[index]
        if token.kind != IDENT or token.text not in _MODIFIERS[self.language]:
            return None
        after = index + 1
        if after < len(toks) and toks[after].text == "(" and token.text in ("pub", "private", "fileprivate",
                                                                             "internal", "public", "package", "open"):
            return self.pairs[after] + 1
        if token.text == "extern" and after < len(toks) and toks[after].kind == LITERAL:
            return after + 1
        return after

    def _body_end(self, keyword: int) -> int | None:
        """The index of the last token of the function's body, or None when it has none."""
        toks, pairs, language = self.toks, self.pairs, self.language
        index = keyword + 1
        while index < len(toks):
            token = toks[index]
            if token.kind == PUNCT:
                if token.text == "{":
                    return pairs[index]
                if token.text in ("(", "["):
                    index = pairs[index] + 1
                    continue
                if token.text in (";", "}", ")", "]"):
                    return None
                if token.text == "@" and language != "rust":
                    return None
                if token.text == "=" and language != "rust" and self._assignment(index):
                    return self._expression_end(index) if language == "kotlin" else None
            elif token.kind == IDENT and index > keyword + 1 and token.text in _DECLARATIONS[language]:
                return None
            elif token.kind == IDENT and token.text in _MODIFIERS[language] and language != "rust":
                if token.line > toks[index - 1].end:
                    return None
            index += 1
        return None

    def _assignment(self, index: int) -> bool:
        toks = self.toks
        before = toks[index - 1].text if index > 0 else ""
        after = toks[index + 1].text if index + 1 < len(toks) else ""
        return before not in _NOT_ASSIGNMENT and after not in ("=", ">")

    def _expression_end(self, equals: int) -> int:
        """The index of the last token of the Kotlin expression body after the `=` at `equals`."""
        toks, pairs = self.toks, self.pairs
        index = equals
        while True:
            token = toks[index]
            last = pairs[index] if token.kind == PUNCT and token.text in "([{" and index in pairs else index
            following = last + 1
            if following >= len(toks):
                return last
            ending, coming = toks[last], toks[following]
            if coming.kind == COMMENT and coming.line == ending.end:
                # a trailing comment: what follows it decides
                following += 1
                if following >= len(toks):
                    return last
                coming = toks[following]
            if ending.text == ";":
                return last - 1
            if coming.line > ending.end and not self._continues(ending, coming):
                return last
            index = following

    @staticmethod
    def _continues(ending: Token, coming: Token) -> bool:
        if coming.kind == COMMENT:
            return True
        if ending.kind == PUNCT and ending.text in _ENDS_CONTINUING:
            return True
        if ending.kind == IDENT and ending.text in _ENDS_CONTINUING_WORDS:
            return True
        if coming.kind == PUNCT and coming.text in _STARTS_CONTINUING:
            return True
        return coming.kind == IDENT and coming.text in _STARTS_CONTINUING_WORDS

    def _name(self, keyword: int) -> str:
        """The function's name: Rust's and Swift's word after the keyword, Kotlin's last word before its parameters
        (`fun <T> Receiver.name(`); an initializer's and a subscript's keyword."""
        toks = self.toks
        word = toks[keyword].text
        if word in ("init", "deinit", "subscript", "constructor"):
            return word
        if self.language == "kotlin":
            index, name = keyword + 1, ""
            while index < len(toks) and toks[index].text not in ("(", "{", "="):
                if toks[index].kind == IDENT:
                    name = toks[index].text
                index += 1
            return name.strip("`")
        after = toks[keyword + 1] if keyword + 1 < len(toks) else None
        if after is not None and after.kind == IDENT:
            return after.text.removeprefix("r#").strip("`")
        operator = ""
        for token in toks[keyword + 1 :]:
            if token.kind != PUNCT or token.text in ("(", "<"):
                break
            operator += token.text
        return operator or word
