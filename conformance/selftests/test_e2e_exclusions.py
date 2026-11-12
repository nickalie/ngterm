"""The `// e2e-coverage:` exclusion markers (agterm_conformance/e2e_exclusions.py): where they are, what each one
excludes, and the markers that fail the report."""

import pytest
from agterm_conformance import e2e_exclusions
from agterm_conformance.e2e_exclusions import FILE, FUNCTION, MarkerError, find_markers


def only(source: str, language: str = "rust") -> e2e_exclusions.Marker:
    markers = find_markers(source, language, "src/a")
    assert len(markers) == 1, markers
    return markers[0]


def refused(source: str, language: str = "rust") -> str:
    with pytest.raises(MarkerError) as error:
        find_markers(source, language, "src/a")
    return str(error.value)


@pytest.mark.parametrize(
    "path, language",
    [("crates/a/src/lib.rs", "rust"), ("app/src/main/java/A.kt", "kotlin"), ("build.gradle.kts", "kotlin"),
     ("NgTerm/App.swift", "swift"), ("README.md", None), ("a/rs", None)],
)
def test_the_language_comes_from_the_file_name(path, language):
    assert e2e_exclusions.language_of(path) == language


def test_a_rust_marker_resolves_to_the_function_below_its_attributes():
    marker = only(
        "fn before() {}\n"
        "// e2e-coverage: off: the crash handler runs only when the process faults\n"
        "#[cfg(unix)]\n"
        "#[allow(\n"
        "    unused\n"
        ")]\n"
        "pub(crate) unsafe extern \"C\" fn on_fault(\n"
        "    signal: i32,\n"
        ") -> Option<u8> where u8: Copy {\n"
        "    let s = \"}\";\n"
        "    None\n"
        "}\n"
        "fn after() {}\n"
    )
    assert (marker.scope, marker.name, marker.line, marker.first, marker.last) == (FUNCTION, "on_fault", 2, 3, 12)
    assert marker.reason == "the crash handler runs only when the process faults"
    assert marker.covers(7) and marker.covers(12) and not marker.covers(2) and not marker.covers(13)


def test_comments_may_sit_between_a_marker_and_its_function():
    marker = only("// e2e-coverage: off: a platform callback\n/// Called by the OS.\n#[inline]\nfn callback() {\n}\n")
    assert (marker.name, marker.first, marker.last) == ("callback", 2, 5)


def test_a_rust_method_in_an_impl_resolves():
    marker = only(
        "impl Thing {\n"
        "    // e2e-coverage:off:   reached only from the macOS app delegate  \n"
        "    pub async fn delegate(&self) {\n"
        "        let _ = |x: u8| { x };\n"
        "    }\n"
        "}\n"
    )
    assert (marker.name, marker.first, marker.last) == ("delegate", 3, 5)
    assert marker.reason == "reached only from the macOS app delegate"


def test_a_file_marker_excludes_the_whole_file():
    marker = only("\n\n// e2e-coverage: off-file: the Android app's JNI entry points\n//! docs\nfn a() {}\n")
    assert (marker.scope, marker.line, marker.reason) == (FILE, 3, "the Android app's JNI entry points")
    assert marker.covers(1) and marker.covers(10_000)


@pytest.mark.parametrize(
    "source, message",
    [
        ("// e2e-coverage: off:\nfn a() {}\n", "src/a:1: a marker with no reason"),
        ("// e2e-coverage: off:   \nfn a() {}\n", "src/a:1: a marker with no reason"),
        ("// e2e-coverage: off-file:\nfn a() {}\n", "src/a:1: a marker with no reason"),
        ("// e2e-coverage: of: typo\nfn a() {}\n", "src/a:1: not `// e2e-coverage: off: <reason>`"),
        ("// e2e-coverage: off because\nfn a() {}\n", "src/a:1: not `// e2e-coverage: off: <reason>`"),
        ("fn a() {}\n// e2e-coverage: off-file: late\n", "src/a:2: an off-file marker must be the file's first"),
        ("// e2e-coverage: off: why\nstruct A;\n", "src/a:1: no function directly below the marker"),
        ("// e2e-coverage: off: why\n\nfn a() {}\n", "src/a:1: no function directly below the marker"),
        ("// e2e-coverage: off: why\n", "src/a:1: no function directly below the marker"),
        ("trait T {\n    // e2e-coverage: off: why\n    fn a();\n}\n", "src/a:2: the function below has no body"),
        ("fn a() {} // e2e-coverage: off: why\nfn b() {}\n", "src/a:1: a marker shares its line with code"),
    ],
)
def test_a_bad_rust_marker_is_refused_naming_its_line(source, message):
    assert message in refused(source)


def test_every_bad_marker_of_a_file_is_named_at_once():
    message = refused("// e2e-coverage: off:\nfn a() {}\n// e2e-coverage: off: why\nstruct B;\n")
    assert "src/a:1: a marker with no reason" in message and "src/a:3: no function directly below" in message


@pytest.mark.parametrize(
    "source",
    [
        'const A: &str = "\n// e2e-coverage: off: in a string\n";\nfn a() {}\n',
        'const A: &str = r#"\n// e2e-coverage: off: in a raw string\n"#;\nfn a() {}\n',
        "/*\n// e2e-coverage: off: in a block comment\n*/\nfn a() {}\n",
        "/* /* nested */\n// e2e-coverage: off: still in the comment\n*/\nfn a() {}\n",
        "/// e2e-coverage: off: a doc comment that mentions the form\nfn a() {}\n",
        "//! e2e-coverage: off-file: an inner doc comment\nfn a() {}\n",
        "// see `// e2e-coverage: off: <reason>` in the docs\nfn a() {}\n",
    ],
)
def test_a_marker_inside_a_string_or_another_comment_is_ignored(source):
    assert find_markers(source, "rust", "src/a") == []


def test_a_rust_source_the_lexer_cannot_read_is_refused():
    assert "src/a: cannot read" in refused('// e2e-coverage: off: why\nfn a() { let s = "unterminated; }\n')


# Kotlin


def test_a_kotlin_marker_resolves_past_annotations_and_modifiers():
    marker = only(
        "class A {\n"
        "    // e2e-coverage: off: Android calls it when the system kills the service\n"
        "    @Suppress(\"UNUSED\")\n"
        "    @get:JvmName(\"x\")\n"
        "    override suspend fun <T : Any> onTrimMemory(level: Int = 1): Map<String, T> {\n"
        "        val c = '{'\n"
        "        val s = \"${ \"}\" + '}' }\"\n"
        "        val r = \"\"\"\n"
        "            }\n"
        "        \"\"\"\n"
        "        return emptyMap()\n"
        "    }\n"
        "    fun after() {}\n"
        "}\n",
        "kotlin",
    )
    assert (marker.name, marker.first, marker.last) == ("onTrimMemory", 3, 12)


def test_a_kotlin_expression_body_runs_to_the_end_of_its_expression():
    marker = only(
        "// e2e-coverage: off: why\n"
        "private fun a(x: Int) = listOf(\n"
        "    x,\n"
        ").map { it + 1 }\n"
        "    .filter { it > 0 }\n"
        "    ?: emptyList()\n"
        "fun b() = 2\n",
        "kotlin",
    )
    assert (marker.name, marker.first, marker.last) == ("a", 2, 6)


def test_a_kotlin_one_line_expression_body_is_its_line():
    marker = only("// e2e-coverage: off: why\nfun a() = 1\nfun b() = 2\n", "kotlin")
    assert (marker.first, marker.last) == (2, 2)


@pytest.mark.parametrize(
    "decl",
    [
        "fun ids(): List<Int> = listOf(1, 2)",
        "fun map(): Map<String, List<Int>> = mapOf()",
        "fun <T> pick(x: T): T where T : Comparable<T> = x",
        "fun f(): (Int) -> Int = { it }",
    ],
)
@pytest.mark.parametrize("next_line", ["fun g() {}\n}", "val y = run { 1 }\n}", "}"])
def test_a_kotlin_expression_body_after_a_generic_return_type_resolves(decl, next_line):
    marker = only(f"class A {{\n// e2e-coverage: off: why\n{decl}\n{next_line}\n", "kotlin")
    assert (marker.first, marker.last) == (3, 3)


@pytest.mark.parametrize(
    "decl, last",
    [
        ("fun identity(to: Int) = to", 2),
        ("fun both(and: Boolean) = and", 2),
        ("fun either(x: Int, or: Int) = x + or", 2),
        ("fun member(x: Pair<Int, Int>) = x.to", 2),
        ("fun pair(x: Int) = x to\n    1", 3),
        ("fun cast(x: Any) = x as\n    String", 3),
        ("fun has(x: Int) = x !in\n    listOf(1)", 3),
        ("fun pick(c: Boolean) = if (c) 1 else\n    2", 3),
    ],
)
def test_a_kotlin_expression_body_ending_in_a_word_stops_at_an_operand(decl, last):
    marker = only(f"// e2e-coverage: off: why\n{decl}\nfun uncovered(): Int {{ return 0 }}\n", "kotlin")
    assert (marker.first, marker.last) == (2, last)


def test_a_kotlin_word_operator_never_continues_into_the_next_declaration():
    marker = only("// e2e-coverage: off: why\nfun a(x: Int) = x to\nprivate fun b() = 2\n", "kotlin")
    assert marker.last == 2


def test_a_kotlin_constructor_and_init_block_resolve():
    markers = find_markers(
        "class A(x: Int) {\n"
        "    // e2e-coverage: off: one\n"
        "    constructor() : this(1) {\n"
        "    }\n"
        "    // e2e-coverage: off: two\n"
        "    init {\n"
        "    }\n"
        "}\n",
        "kotlin",
        "A.kt",
    )
    assert [(marker.name, marker.first, marker.last) for marker in markers] == [
        ("constructor", 3, 4), ("init", 6, 7)
    ]


def test_a_kotlin_delegating_constructor_without_a_body_runs_to_its_call():
    marker = only(
        "class A(x: Int) {\n"
        "    // e2e-coverage: off: why\n"
        "    constructor(s: String) : this(\n"
        "        s.length,\n"
        "    )\n"
        "    fun after() {}\n"
        "}\n",
        "kotlin",
    )
    assert (marker.name, marker.first, marker.last) == ("constructor", 3, 5)


@pytest.mark.parametrize(
    "source, message",
    [
        ("interface I {\n    // e2e-coverage: off: why\n    fun a(): Int\n    fun b()\n}\n", "has no body"),
        ("// e2e-coverage: off: why\nval x = 1\n", "no function directly below"),
        ("// e2e-coverage: off: why\nclass A {}\n", "no function directly below"),
    ],
)
def test_a_bad_kotlin_marker_is_refused(source, message):
    assert message in refused(source, "kotlin")


@pytest.mark.parametrize(
    "source",
    [
        'val a = "${ "// e2e-coverage: off: in a template" }"\nfun a() {}\n',
        'val a = """\n// e2e-coverage: off: in a raw string\n"""\nfun a() {}\n',
        "/* /*\n*/\n// e2e-coverage: off: in a nested comment\n*/\nfun a() {}\n",
    ],
)
def test_a_kotlin_marker_in_a_string_or_comment_is_ignored(source):
    assert find_markers(source, "kotlin", "A.kt") == []


# Swift


def test_a_swift_marker_resolves_past_attributes_and_modifiers():
    marker = only(
        "final class A {\n"
        "    // e2e-coverage: off: iOS calls it on a memory warning only\n"
        "    @MainActor\n"
        "    @available(iOS 17, *)\n"
        "    private(set) nonisolated static func warn(_ x: Int = 1) async throws -> [String: Int] {\n"
        "        let s = \"\\(x + [1].map { $0 }.count) }\"\n"
        "        let r = #\"a \"} \\(x)\"#\n"
        "        let m = \"\"\"\n"
        "            }\n"
        "            \"\"\"\n"
        "        return [:]\n"
        "    }\n"
        "    func after() {}\n"
        "}\n",
        "swift",
    )
    assert (marker.name, marker.first, marker.last) == ("warn", 3, 12)


@pytest.mark.parametrize(
    "decl, name",
    [("init?(x: Int) {\n}", "init"), ("deinit {\n}", "deinit"), ("subscript(i: Int) -> Int {\n    0\n}", "subscript"),
     ("var computed: Int {\n    1\n}", "computed")],
)
def test_swift_initializers_subscripts_and_computed_properties_resolve(decl, name):
    marker = only(f"// e2e-coverage: off: why\n{decl}\n", "swift")
    assert (marker.name, marker.first, marker.last) == (name, 2, 1 + decl.count("\n") + 1)


@pytest.mark.parametrize("op", ["<", "<=", "<<", "<<=", "==", "+"])
def test_a_swift_operator_function_is_named_by_its_operator(op):
    marker = only(f"// e2e-coverage: off: why\nstatic func {op} (lhs: A, rhs: A) -> Bool {{\n}}\n", "swift")
    assert marker.name == op


def test_a_generic_swift_operator_function_is_named_by_its_operator():
    marker = only("// e2e-coverage: off: why\nstatic func < <T>(lhs: T, rhs: T) -> Bool {\n}\n", "swift")
    assert marker.name == "<"


@pytest.mark.parametrize(
    "source, message",
    [
        ("protocol P {\n    // e2e-coverage: off: why\n    func a()\n    func b()\n}\n", "has no body"),
        ("protocol P {\n    // e2e-coverage: off: why\n    var x: Int { get set }\n}\n", "has no body"),
        ("protocol P {\n    // e2e-coverage: off: why\n    var x: Int { get async throws(E) }\n}\n", "has no body"),
        ("protocol P {\n    // e2e-coverage: off: why\n    subscript(i: Int) -> Int { get }\n}\n", "has no body"),
        ("// e2e-coverage: off: why\nvar stored: Int = 1\n", "has no body"),
        ("// e2e-coverage: off: why\nstruct S {}\n", "no function directly below"),
    ],
)
def test_a_bad_swift_marker_is_refused(source, message):
    assert message in refused(source, "swift")


@pytest.mark.parametrize(
    "source",
    [
        'let a = "\\("// e2e-coverage: off: in an interpolation")"\nfunc a() {}\n',
        'let a = #"" // e2e-coverage: off: in a raw string"#\nfunc a() {}\n',
        'let a = #"""\n// e2e-coverage: off: in a multi-line raw string\n"""#\nfunc a() {}\n',
        'let a = """\n// e2e-coverage: off: in a multi-line string\n"""\nfunc a() {}\n',
    ],
)
def test_a_swift_marker_in_a_string_is_ignored(source):
    assert find_markers(source, "swift", "A.swift") == []


def test_a_file_without_the_marker_text_is_not_lexed():
    # a source no lexer reads, but with no marker in it, has nothing to find
    assert find_markers('fn a() { let s = "unterminated; }\n', "rust", "src/a") == []
