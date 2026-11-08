use super::*;

#[test]
fn source_and_picture_index_are_inert_including_combining_marks() {
    let source = "</pre><script>alert(1)</script> <img src=x onerror=y> & \"q\" 'a' <\u{301}b>";
    let images = Embedded {
        references: BTreeMap::from([("x\"\u{301} data-test=1.png".to_owned(), 0)]),
        uris: vec!["data:image/png;base64,AA==".to_owned()],
    };
    let document = html(source, &images, "p{}");
    assert!(!document.contains("<script>"));
    assert!(!document.contains("<img"));
    assert!(document.contains("&lt;/pre&gt;&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(document.contains("&amp; &quot;q&quot; &#39;a&#39; &lt;\u{301}b&gt;"));
    assert!(document.contains("x\\&quot;\u{301} data-test=1.png"));
    assert!(document.contains("default-src 'none'; img-src data:; style-src 'unsafe-inline'"));
    assert!(document.contains("<style>p{}</style>"));
}

#[test]
fn inline_reference_spaced_and_unicode_pictures_resolve_once_per_decoded_reference() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    let markdown = "![a](img/shot.png) ![b](./img/shot.png) ![c][r]\n\n[r]: ref.jpg\n\n\
        ![d](<my shot.png>) ![e](my%20shot.png) ![f](café.png)";
    let found = image_references(markdown, root);
    let names: Vec<_> = found.iter().map(|image| image.reference.as_str()).collect();
    assert_eq!(
        names,
        ["img/shot.png", "./img/shot.png", "ref.jpg", "my shot.png", "café.png"]
    );
    for image in found {
        assert!(image.path.starts_with(root));
    }
}

#[test]
fn lexical_escape_and_schemes_are_rejected_before_probing() {
    let folder = tempfile::tempdir().unwrap();
    for reference in [
        "https://x.io/a.png",
        "data:image/png;base64,AA==",
        "../outside.png",
        "~/a.png",
        "/etc/hosts",
        "img/missing/../../..",
        "file:///tmp/a.png",
    ] {
        assert!(local_path(reference, folder.path()).is_none(), "{reference}");
    }
    let inside = folder.path().join("img/shot.png");
    assert_eq!(local_path(&inside.to_string_lossy(), folder.path()), Some(inside));
    assert_eq!(MAX_IMAGE_PIXELS, super::super::image_page::MAX_PIXELS);
    assert_eq!(MAX_BYTES, 4 << 20);
    assert_eq!(MAX_IMAGE_BYTES, 32 << 20);
}

#[test]
fn invalid_percent_encoding_stays_a_literal_picture_reference() {
    let folder = tempfile::tempdir().unwrap();
    let found = image_references("![a](%FF.png)", folder.path());
    assert_eq!(
        found,
        vec![ImageReference {
            reference: "%FF.png".to_owned(),
            path: folder.path().join("%FF.png"),
        }]
    );
}
