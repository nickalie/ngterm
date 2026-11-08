//! Upstream's `DocumentPageTests.swift`, and the Chromium page's link handling.

use super::*;

/// Upstream's `thePolicyGoesRightAfterALeadingDoctypeOrFirst`: `§` marks where the policy goes.
#[test]
fn the_policy_goes_right_after_a_leading_doctype_or_first() {
    for (html, expected) in [
        ("<h1>héllo</h1>", "§<h1>héllo</h1>"),
        ("<!DOCTYPE html><title>t</title>", "<!DOCTYPE html>§<title>t</title>"),
        ("\u{FEFF}\n  <!doctype html>\n<p>x", "\n  <!doctype html>§\n<p>x"),
        ("<!-- c --><!doctype html>", "§<!-- c --><!doctype html>"),
        ("<!doctype html", "§<!doctype html"),
        ("\u{A0}<!doctype html><p>x", "§\u{A0}<!doctype html><p>x"),
        ("\r\n\t<!doctype html>", "\r\n\t<!doctype html>§"),
        ("<!doctype html>\u{0301}<p>x</p>", "<!doctype html>§\u{0301}<p>x</p>"),
        (
            "\u{c}<!DocType html SYSTEM \"about:legacy-compat\">x",
            "\u{c}<!DocType html SYSTEM \"about:legacy-compat\">§x",
        ),
        ("", "§"),
    ] {
        assert_eq!(
            page(html.as_bytes().to_vec()),
            Some(expected.replace('§', POLICY)),
            "{html:?}"
        );
    }
}

/// Upstream's `binaryOrNonUtf8IsRefused`.
#[test]
fn binary_or_non_utf8_is_refused() {
    for data in [vec![0x3C, 0x68, 0x00, 0x3E], vec![0xFF, 0xFE, 0x3C]] {
        assert_eq!(page(data.clone()), None);
        assert_eq!(chromium_page(data), None);
    }
}

/// The policy loads only `data:` and inline styles: no script, frame, connection or remote subresource.
#[test]
fn the_policy_loads_only_data_and_inline_styles() {
    assert_eq!(
        POLICY,
        "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data:; style-src \
         'unsafe-inline'; font-src data:; media-src data:\">"
    );
}

/// A Chromium engine preconnects and prefetches DNS for link hints past the CSP and the request filter: every `<link>`
/// start tag, in any case, goes inert, while a longer tag name or a closing tag stays.
#[test]
fn a_chromium_page_has_no_live_link_elements() {
    let html = "<!doctype html><LINK rel=preconnect href=\"http://127.0.0.1:1\"><link\trel=dns-prefetch \
                href=//h><link/rel=prefetch href=x><link><linkx a><a href=x>link</a></link><p><link";
    let built = chromium_page(html.as_bytes().to_vec()).unwrap();
    assert_eq!(
        built,
        format!(
            "<!doctype html>{POLICY}{NO_DNS_PREFETCH}<ngterm-inert-link rel=preconnect href=\"http://127.0.0.1:1\">\
             <ngterm-inert-link\trel=dns-prefetch href=//h><ngterm-inert-link/rel=prefetch href=x>\
             <ngterm-inert-link><linkx a><a href=x>link</a></link><p><ngterm-inert-link"
        )
    );
    assert!(!built.to_ascii_lowercase().contains("<link "));
}

/// Text around the renamed tags, multibyte included, is kept byte for byte.
#[test]
fn renaming_links_keeps_the_rest_of_the_text() {
    assert_eq!(without_links("é<link>ü"), "é<ngterm-inert-link>ü");
    assert_eq!(without_links("no tags here"), "no tags here");
    assert_eq!(without_links("<lin"), "<lin");
}
