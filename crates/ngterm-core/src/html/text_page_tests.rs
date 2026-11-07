//! Upstream's `TextPageTests.swift`.

use super::*;

#[test]
fn the_text_stays_inert_and_the_policy_loads_nothing() {
    let source = "</pre><script>alert(1)</script> <img src=x onerror=y> <iframe src=a> & \"q\" 'a'";
    let html = html(source, Some("swift"), Some(3), "p{}");
    assert!(!html.contains("<script>"));
    assert!(!html.contains("<img"));
    assert!(!html.contains("<iframe"));
    assert!(html.contains(
        "&lt;/pre&gt;&lt;script&gt;alert(1)&lt;/script&gt; &lt;img src=x onerror=y&gt; &lt;iframe src=a&gt; &amp; \
         &quot;q&quot; &#39;a&#39;"
    ));
    assert!(html.contains("content=\"default-src 'none'; style-src 'unsafe-inline'\""));
    assert!(!html.contains("img-src"));
    assert!(html.contains("<div id=\"ngterm-text\" hidden data-language=\"swift\" data-line=\"3\">"));
    assert!(html.contains("<style>p{}</style>"));
}

// regression (upstream): a quote or bracket fused with a combining mark is one Character and skipped escaping
#[test]
fn a_metacharacter_followed_by_a_combining_mark_is_still_escaped() {
    let html = html("<\u{301}b> \"\u{301}", None, None, "");
    assert!(html.contains("&lt;\u{301}b&gt; &quot;\u{301}"));
}

#[test]
fn no_language_or_line_leaves_the_attributes_out() {
    assert!(html("x", None, None, "").contains("<div id=\"ngterm-text\" hidden>x</div>"));
}

// regression (upstream): a newline right after <pre> is dropped by the parser and shifted every line number
#[test]
fn a_leading_blank_line_survives_in_the_holder() {
    let html = html("\n\nfunc a()", None, None, "");
    assert!(html.contains("<div id=\"ngterm-text\" hidden>\n\nfunc a()</div>"));
    assert!(!html.contains("<pre"));
}

#[test]
fn text_over_the_highlight_cap_renders_plain() {
    let at_cap = "a".repeat(MAX_HIGHLIGHT_BYTES);
    let over_cap = format!("{at_cap}é");
    assert!(html(&at_cap, Some("go"), None, "").contains("data-language=\"go\""));
    assert!(!html(&over_cap, Some("go"), None, "").contains("data-language"));
}

#[test]
fn mapped_files_get_their_language() {
    let cases = [
        ("a.swift", "swift"),
        ("a.go", "go"),
        ("a.py", "python"),
        ("a.rb", "ruby"),
        ("a.rs", "rust"),
        ("a.js", "javascript"),
        ("a.mjs", "javascript"),
        ("a.cjs", "javascript"),
        ("a.ts", "typescript"),
        ("a.c", "c"),
        ("a.h", "c"),
        ("a.cc", "cpp"),
        ("a.cpp", "cpp"),
        ("a.cxx", "cpp"),
        ("a.hpp", "cpp"),
        ("a.m", "objectivec"),
        ("a.mm", "objectivec"),
        ("a.java", "java"),
        ("a.kt", "kotlin"),
        ("a.kts", "kotlin"),
        ("a.cs", "csharp"),
        ("a.php", "php"),
        ("a.pl", "perl"),
        ("a.pm", "perl"),
        ("a.lua", "lua"),
        ("a.r", "r"),
        ("a.sql", "sql"),
        ("a.css", "css"),
        ("a.scss", "scss"),
        ("a.less", "less"),
        ("a.graphql", "graphql"),
        ("a.sh", "bash"),
        ("a.bash", "bash"),
        ("a.zsh", "bash"),
        ("a.json", "json"),
        ("a.yaml", "yaml"),
        ("a.yml", "yaml"),
        ("a.toml", "ini"),
        ("a.ini", "ini"),
        ("a.cfg", "ini"),
        ("a.conf", "ini"),
        ("a.xml", "xml"),
        ("a.plist", "xml"),
        ("a.diff", "diff"),
        ("a.patch", "diff"),
        ("a.mk", "makefile"),
        ("a.vb", "vbnet"),
        ("/x/Makefile", "makefile"),
        ("/x/GNUmakefile", "makefile"),
        ("/x/.bashrc", "bash"),
        ("/x/.zshrc", "bash"),
        ("/x/.profile", "bash"),
        ("/x/.bash_profile", "bash"),
        ("/x/.zprofile", "bash"),
        ("/x/.gitconfig", "ini"),
        ("/x/.editorconfig", "ini"),
        ("/x/MAIN.GO", "go"),
        ("/x/y/Lib.Swift", "swift"),
    ];
    for (path, expected) in cases {
        assert_eq!(language(path), Some(expected), "{path}");
        assert!(is_eligible(path), "{path}");
    }
}

#[test]
fn plain_files_are_eligible_with_no_language() {
    for path in [
        "a.txt",
        "a.TEXT",
        "a.log",
        "a.out",
        "a.csv",
        "a.tsv",
        "a.env",
        "/x/Cargo.lock",
        "/x/Dockerfile",
        "/x/LICENSE",
        "/x/README",
        "/x/CHANGELOG",
        "/x/Procfile",
        "/x/.gitignore",
        "/x/.env",
    ] {
        assert_eq!(language(path), None, "{path}");
        assert!(is_eligible(path), "{path}");
    }
}

#[test]
fn other_files_are_not_text_pages() {
    for path in [
        "a.md",
        "a.markdown",
        "a.html",
        "a.htm",
        "a.svg",
        "a.png",
        "a.pdf",
        "a.app",
        "a.command",
        "a.zip",
        "a.wasm",
        "/x/makefile",
        "/x/license",
        "/x/noext",
        "/x/dir/",
        "",
    ] {
        assert!(!is_eligible(path), "{path}");
    }
}

#[test]
fn a_windows_path_is_named_by_its_last_component() {
    if cfg!(windows) {
        assert_eq!(language("C:\\src\\main.go"), Some("go"));
        assert!(is_eligible("C:\\src\\Makefile"));
    }
}

#[test]
fn validate_accepts_utf8_and_refuses_binaries() {
    assert_eq!(validate("héllo\n".as_bytes().to_vec()).as_deref(), Some("héllo\n"));
    assert_eq!(validate(Vec::new()).as_deref(), Some(""));
    assert!(validate(vec![0xEF, 0xBB, 0xBF, 0x61]).is_some());
    assert_eq!(validate(vec![0x61, 0x00, 0x62]), None);
    assert_eq!(validate(vec![0xFF, 0xFE, 0x61]), None);
    assert_eq!(validate(vec![0x61, 0xC3]), None);
}
