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

// regression (upstream #897): the parser drops a newline right after `<pre>`, so a rule on line two read as frontmatter
#[test]
fn the_holder_is_a_div_that_keeps_a_leading_blank_line() {
    let document = html("\n---\n## Heading", &Embedded::default(), "");
    assert!(
        document.contains("<div id=\"ngterm-markdown\" hidden data-images=\"{&quot;references&quot;:{},&quot;uris&quot;:[]}\">\n---\n## Heading</div>"),
        "{document}"
    );
    assert!(!document.contains("<pre"));
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
        assert!(lexical_path(reference, folder.path()).is_none(), "{reference}");
        assert!(local_path(reference, folder.path()).is_none(), "{reference}");
    }
    let inside = folder.path().join("img/shot.png");
    assert_eq!(lexical_path(&inside.to_string_lossy(), folder.path()), Some(inside));
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

/// Upstream's `lexicalPathJoinsAndNormalizesInsideTheFolder` and `lexicalPathRefusesWhatLeavesTheFolder`.
#[test]
fn lexical_path_joins_and_normalizes_inside_the_folder_and_refuses_what_leaves_it() {
    let folder = PathBuf::from(super::super::os_path("/f"));
    for (reference, expected) in [("a.png", "/f/a.png"), ("img/../b.png", "/f/b.png")] {
        let expected = PathBuf::from(super::super::os_path(expected));
        assert_eq!(lexical_path(reference, &folder), Some(expected), "{reference}");
    }
    let absolute = super::super::os_path("/f/c.png");
    assert_eq!(lexical_path(&absolute, &folder), Some(PathBuf::from(&absolute)));
    for reference in ["../a.png", "~/a.png", "https://x.io/a.png", "", "."] {
        assert_eq!(lexical_path(reference, &folder), None, "{reference}");
    }
    assert_eq!(lexical_path(&super::super::os_path("/etc/hosts"), &folder), None);
}

/// Upstream's `remoteReferencesIgnoreSymlinksOnThisMac`: the listing a viewer makes of another machine's file is
/// lexical, so a link here neither drops nor moves a reference; the symlink-resolving [`local_path`] the origin serves
/// pictures by refuses one that leaves the folder.
#[cfg(unix)]
#[test]
fn listed_references_ignore_symlinks_here_and_local_paths_resolve_them() {
    let outside = tempfile::tempdir().unwrap();
    let shot = outside.path().join("out.png");
    std::fs::write(&shot, "x").unwrap();
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    std::os::unix::fs::symlink(&shot, root.join("out.png")).unwrap();
    std::fs::write(root.join("in.png"), "x").unwrap();

    let markdown = "![o](out.png) ![m](img/missing.png) ![e](../up.png)";
    let found = image_references(markdown, root);
    let paths: Vec<_> = found.iter().map(|image| image.path.clone()).collect();
    assert_eq!(paths, [root.join("out.png"), root.join("img/missing.png")]);
    let remote = remote_image_references(markdown, &root.join("README.md").to_string_lossy());
    let remote: Vec<_> = remote.iter().map(|image| PathBuf::from(&image.path)).collect();
    assert_eq!(
        remote, paths,
        "a listing of another machine's file ignores links here too"
    );

    assert_eq!(local_path("out.png", root), None);
    let real_root = root.canonicalize().unwrap();
    assert_eq!(local_path("in.png", root), Some(real_root.join("in.png")));
    assert_eq!(
        local_path("img/missing.png", root),
        Some(real_root.join("img/missing.png"))
    );
    assert_eq!(local_path(".", root), None);
}
