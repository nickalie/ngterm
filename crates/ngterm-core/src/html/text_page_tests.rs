//! Upstream's `TextPageTests.swift`.

use super::*;

#[test]
fn the_text_stays_inert_and_the_policy_loads_nothing() {
    let source = "</pre><script>alert(1)</script> <img src=x onerror=y> <iframe src=a> & \"q\" 'a'";
    let html = html(source, Some("swift"), Some(3), "p{}", &PageState::default());
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
    let html = html("<\u{301}b> \"\u{301}", None, None, "", &PageState::default());
    assert!(html.contains("&lt;\u{301}b&gt; &quot;\u{301}"));
}

#[test]
fn no_language_or_line_leaves_the_attributes_out() {
    assert!(html("x", None, None, "", &PageState::default()).contains("<div id=\"ngterm-text\" hidden>x</div>"));
}

// regression (upstream): a newline right after <pre> is dropped by the parser and shifted every line number
#[test]
fn a_leading_blank_line_survives_in_the_holder() {
    let html = html("\n\nfunc a()", None, None, "", &PageState::default());
    assert!(html.contains("<div id=\"ngterm-text\" hidden>\n\nfunc a()</div>"));
    assert!(!html.contains("<pre"));
}

#[test]
fn text_over_the_highlight_cap_renders_plain() {
    let at_cap = "a".repeat(MAX_HIGHLIGHT_BYTES);
    let over_cap = format!("{at_cap}é");
    assert!(html(&at_cap, Some("go"), None, "", &PageState::default()).contains("data-language=\"go\""));
    assert!(!html(&over_cap, Some("go"), None, "", &PageState::default()).contains("data-language"));
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

/// Upstream's `onlyLogAndOutFilesAreLogs`.
#[test]
fn only_log_and_out_files_are_logs() {
    for (path, log) in [
        ("a.log", true),
        ("/x/y/APP.LOG", true),
        ("a.out", true),
        ("a.txt", false),
        ("a.log.gz", false),
        ("/x/log", false),
        ("a.swift", false),
    ] {
        assert_eq!(is_log(path), log, "{path}");
    }
}

/// Upstream's `aLogPageCarriesItsStateInsteadOfALine`.
#[test]
fn a_log_page_carries_its_state_instead_of_a_line() {
    let log = TextLogState {
        following: true,
        report: TextLogReport {
            lines: 2,
            clipped: true,
            waiting: true,
            unreadable: true,
        },
    };
    let state = PageState {
        log: Some(&log),
        anchor: None,
        ..PageState::default()
    };
    let html = html("a\nb\n", None, Some(7), "", &state);
    assert!(html.contains(
        "<div id=\"ngterm-text\" hidden data-log data-following=\"true\" data-clipped data-waiting data-unreadable>"
    ));
    assert!(!html.contains("data-line"));
}

/// Upstream's `aPausedLogPageOmitsTheUnsetFlags`.
#[test]
fn a_paused_log_page_omits_the_unset_flags() {
    let log = TextLogState::new(TextLog {
        tail: 50,
        follow: false,
    });
    let state = PageState {
        log: Some(&log),
        anchor: None,
        ..PageState::default()
    };
    assert!(
        html("a", None, None, "", &state)
            .contains("<div id=\"ngterm-text\" hidden data-log data-following=\"false\">a</div>")
    );
}

/// Upstream's `aRebuiltPageCarriesOnlyAFiniteAnchor`, the anchor spelled as Swift's `Double` prints it.
#[test]
fn a_rebuilt_page_carries_only_a_finite_anchor() {
    let paused = TextLogState::default();
    let at = |log: Option<&TextLogState>, line: Option<i64>, anchor: Option<f64>| {
        html(
            "a",
            None,
            line,
            "",
            &PageState {
                log,
                anchor,
                ..PageState::default()
            },
        )
    };
    assert!(at(Some(&paused), None, Some(240.5)).contains("data-anchor=\"240.5\""));
    assert!(at(None, Some(3), Some(80.0)).contains("data-line=\"3\" data-anchor=\"80.0\""));
    assert!(!at(Some(&paused), None, Some(f64::INFINITY)).contains("data-anchor"));
    assert!(!at(Some(&paused), None, Some(f64::NAN)).contains("data-anchor"));
    assert!(!at(None, None, None).contains("data-anchor"));
}

/// The log constants are upstream's.
#[test]
fn the_log_limits_are_upstreams() {
    assert_eq!(
        (DEFAULT_TAIL, MAX_TAIL, MAX_TAIL_BYTES),
        (5_000, 100_000, 2 * 1024 * 1024)
    );
}

/// A built log document carries its build generation for the renderer's ready report to echo (a fix over upstream).
#[test]
fn a_built_log_page_carries_its_load() {
    let log = TextLogState::default();
    let state = PageState {
        log: Some(&log),
        anchor: Some(4.0),
        load: Some(7),
        ..PageState::default()
    };
    assert!(html("a", None, None, "", &state).contains(" data-anchor=\"4.0\" data-load=\"7\">a</div>"));
    assert!(!html("a", None, None, "", &PageState::default()).contains("data-load"));
}

/// Upstream's `aPageCarriesItsSearchWithTheQueryEscaped`.
#[test]
fn a_page_carries_its_search_with_the_query_escaped() {
    let search = SearchState {
        query: "\"><x>".to_owned(),
        revision: 4,
        current: 2,
    };
    let state = PageState {
        search: Some(&search),
        ..PageState::default()
    };
    assert!(html("a", None, None, "", &state).contains(
        "<div id=\"ngterm-text\" hidden data-revision=\"4\" data-query=\"&quot;&gt;&lt;x&gt;\" data-current=\"2\">"
    ));
}

/// Upstream's `aClosedSearchCarriesOnlyItsRevision`.
#[test]
fn a_closed_search_carries_only_its_revision() {
    let closed = SearchState {
        query: String::new(),
        revision: 5,
        current: 0,
    };
    let state = PageState {
        search: Some(&closed),
        ..PageState::default()
    };
    assert!(html("a", None, None, "", &state).contains("<div id=\"ngterm-text\" hidden data-revision=\"5\">"));
    assert!(!html("a", None, None, "", &PageState::default()).contains("data-revision"));
}

fn with_change(change: usize) -> PageState<'static> {
    PageState {
        change: Some(change),
        ..PageState::default()
    }
}

/// Upstream's `aDiffIsMarkedPastTheHighlightCapWithTheChangeItReopensOn`.
#[test]
fn a_diff_is_marked_past_the_highlight_cap_with_the_change_it_reopens_on() {
    let over_cap = "a".repeat(MAX_HIGHLIGHT_BYTES + 1);
    let built = html(&over_cap, Some("diff"), None, "", &with_change(4));
    assert!(built.contains("hidden data-diff data-change=\"4\">"));
    assert!(!built.contains("data-language"));
    assert!(!html("x", Some("diff"), None, "", &with_change(0)).contains("data-change"));
}

/// Upstream's `onlyAPlainDiffPageIsMarked`.
#[test]
fn only_a_plain_diff_page_is_marked() {
    let log = TextLogState {
        following: true,
        ..TextLogState::default()
    };
    let state = PageState {
        log: Some(&log),
        ..with_change(2)
    };
    let built = html("x", Some("diff"), None, "", &state);
    assert!(!built.contains("data-diff"));
    assert!(!built.contains("data-change"));
    assert!(!html("x", Some("go"), None, "", &with_change(2)).contains("data-diff"));
}

/// Upstream's `isDiffFollowsTheExtension`.
#[test]
fn is_diff_follows_the_extension() {
    for (path, expected) in [
        ("a.diff", true),
        ("/x/b.PATCH", true),
        ("a.txt", false),
        ("a.go", false),
        ("diff", false),
    ] {
        assert_eq!(is_diff(path), expected, "{path}");
    }
}

fn languages(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(path, language)| ((*path).to_owned(), (*language).to_owned()))
        .collect()
}

/// Upstream's `aDiffMapsEachHeaderPathToItsLanguage`: up to a tab, a quoted path looked up bare, an unknown language
/// and `/dev/null` left out.
#[test]
fn a_diff_maps_each_header_path_to_its_language() {
    let text = "diff --git a/main.go b/main.go\n--- a/main.go\n+++ b/main.go\n--- /dev/null\n\
                +++ \"b/odd name.py\"\n--- old.sql\t2026-11-06 10:00:00\n+++ notes.unknown";
    assert_eq!(
        diff_languages(text),
        languages(&[
            ("a/main.go", "go"),
            ("b/main.go", "go"),
            ("\"b/odd name.py\"", "python"),
            ("old.sql", "sql"),
        ])
    );
}

/// Upstream's `aCRLFDiffMapsItsHeaderPaths`.
#[test]
fn a_crlf_diff_maps_its_header_paths() {
    assert_eq!(
        diff_languages("diff --git a/x.go b/x.go\r\n--- a/x.go\r\n+++ b/x.go\r\n"),
        languages(&[("a/x.go", "go"), ("b/x.go", "go")])
    );
}

/// Upstream's `aDiffUnderTheHighlightCapCarriesItsFileLanguages`: sorted keys, escaped; not past the cap, on another
/// language or with no header.
#[test]
fn a_diff_under_the_highlight_cap_carries_its_file_languages() {
    let text = "--- a/x.go\n+++ b/x.go\n@@ -1 +1 @@\n-a\n+b\n";
    let over_cap = format!("{text}{}", " x\n".repeat(MAX_HIGHLIGHT_BYTES / 3 + 1));
    let none = PageState::default();
    assert!(
        html(text, Some("diff"), None, "", &none)
            .contains("data-files=\"{&quot;a/x.go&quot;:&quot;go&quot;,&quot;b/x.go&quot;:&quot;go&quot;}\"")
    );
    assert!(!html(&over_cap, Some("diff"), None, "", &none).contains("data-files"));
    assert!(!html(text, Some("go"), None, "", &none).contains("data-files"));
    assert!(!html("-a\n+b\n", Some("diff"), None, "", &none).contains("data-files"));
}
