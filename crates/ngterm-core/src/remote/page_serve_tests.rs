//! Upstream's `PageServeFramesTests`, `RemotePageResourcesTests` and the admission case of
//! `ControlServerPageServeTests`.

use std::path::{Path, PathBuf};

use super::*;

fn frames() -> Vec<PageServeFrame> {
    vec![
        PageServeFrame::Resource {
            id: 1,
            path: "/Users/u/docs/img/a b.png".to_owned(),
        },
        PageServeFrame::Head {
            id: 1,
            mime_type: "image/png".to_owned(),
        },
        PageServeFrame::Chunk {
            id: 1,
            data: vec![0, 1, 2, 255, 10, 13],
        },
        PageServeFrame::End { id: 1 },
        PageServeFrame::Failure {
            id: 2,
            error: "file is outside the page's folder".to_owned(),
        },
        PageServeFrame::Cancel { id: 3 },
        PageServeFrame::Bridge {
            id: 4,
            request: r#"{"cmd":"session.overlay.submit","args":{"value":"done\nwith newline"}}"#.to_owned(),
        },
        PageServeFrame::Bridge {
            id: 5,
            request: r#"{"cmd":"session.future.thing","args":{"newArgument":[1,{"x":true}]}}"#.to_owned(),
        },
        PageServeFrame::Reply {
            id: 5,
            response: r#"{"ok":true,"result":{"id":"s1","futureField":{"nested":1}}}"#.to_owned(),
        },
    ]
}

#[test]
fn every_frame_survives_a_round_trip() {
    for frame in frames() {
        let line = encode(&frame).unwrap();
        assert_eq!(line.last(), Some(&b'\n'));
        let body = &line[..line.len() - 1];
        assert!(!body.contains(&b'\n'), "{frame:?}");
        assert_eq!(decode(body).unwrap(), frame);
    }
}

#[test]
fn a_frame_is_written_with_sorted_keys_and_unescaped_slashes() {
    let line = encode(&PageServeFrame::Resource {
        id: 7,
        path: "/a/b.css".to_owned(),
    })
    .unwrap();
    assert_eq!(line, b"{\"id\":7,\"kind\":\"resource\",\"path\":\"/a/b.css\"}\n");
    let chunk = encode(&PageServeFrame::Chunk {
        id: 1,
        data: b"hi".to_vec(),
    })
    .unwrap();
    assert_eq!(chunk, b"{\"data\":\"aGk=\",\"id\":1,\"kind\":\"chunk\"}\n");
    let head = encode(&PageServeFrame::Head {
        id: 2,
        mime_type: "text/css".to_owned(),
    })
    .unwrap();
    assert_eq!(head, b"{\"id\":2,\"kind\":\"head\",\"mimeType\":\"text/css\"}\n");
}

#[test]
fn a_full_chunk_fits_one_line_and_a_larger_one_is_refused_both_ways() {
    let full = PageServeFrame::Chunk {
        id: i64::MAX,
        data: vec![0xFF; MAX_CHUNK_BYTES],
    };
    let line = encode(&full).unwrap();
    assert!(line.len() < MAX_FRAME_BYTES);
    assert_eq!(decode(&line[..line.len() - 1]).unwrap(), full);

    let over = vec![0xFF; MAX_CHUNK_BYTES + 1];
    assert_eq!(
        encode(&PageServeFrame::Chunk {
            id: 1,
            data: over.clone()
        }),
        Err(FrameError::Oversize(over.len()))
    );
    let wire = format!(r#"{{"data":"{}","id":1,"kind":"chunk"}}"#, STANDARD.encode(&over));
    assert_eq!(decode(wire.as_bytes()), Err(FrameError::Oversize(over.len())));
}

#[test]
fn a_bridge_request_too_large_for_a_line_is_refused() {
    let value = "x".repeat(MAX_FRAME_BYTES);
    let frame = PageServeFrame::Bridge {
        id: 1,
        request: format!(r#"{{"cmd":"session.overlay.submit","args":{{"value":"{value}"}}}}"#),
    };
    assert!(matches!(encode(&frame), Err(FrameError::Oversize(_))));
}

#[test]
fn a_malformed_line_is_refused() {
    for line in [
        "not json",
        r#"{"id":1}"#,
        r#"{"kind":"resource","id":1}"#,
        r#"{"kind":"future","id":1}"#,
        r#"{"kind":"chunk","id":1,"data":"not base64!"}"#,
        "[]",
    ] {
        assert!(
            matches!(decode(line.as_bytes()), Err(FrameError::Malformed(_))),
            "{line}"
        );
    }
}

#[test]
fn an_oversize_line_is_refused_before_decoding() {
    let line = vec![b' '; MAX_FRAME_BYTES + 1];
    assert_eq!(decode(&line), Err(FrameError::Oversize(line.len())));
}

/// A grant `docs` beside a file outside it, with a link inside the grant pointing out on Unix (upstream's fixture).
/// Requests name the files under the temporary directory as the system spells it; what [`resolve`] answers is the
/// canonical path (macOS' `/private/var`, Windows' `\\?\` form), so expectations go through `canonicalize` too.
struct Docs {
    dir: tempfile::TempDir,
}

impl Docs {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir_all(docs.join("img")).unwrap();
        for file in [
            docs.join("index.html"),
            docs.join("img").join("a.png"),
            dir.path().join("secret.txt"),
        ] {
            std::fs::write(file, "x").unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("secret.txt"), docs.join("out.txt")).unwrap();
        Self { dir }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    /// `docs` joined with `parts`, one component each.
    fn docs(&self, parts: &[&str]) -> PathBuf {
        parts
            .iter()
            .fold(self.root().join("docs"), |path, part| path.join(part))
    }

    fn file(&self, grant: bool) -> HtmlSource {
        HtmlSource::File {
            path: text(&self.docs(&["index.html"])),
            grant_root: grant.then(|| text(&self.docs(&[]))),
        }
    }
}

fn text(path: &Path) -> String {
    path.to_str().unwrap().to_owned()
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap()
}

#[test]
fn a_grant_serves_files_inside_it() {
    let docs = Docs::new();
    let source = docs.file(true);
    for path in [docs.docs(&["index.html"]), docs.docs(&["img", "a.png"])] {
        assert_eq!(resolve(&text(&path), &source), Some(canonical(&path)));
    }
    // a file inside the grant that does not exist yet is the file's to refuse, not the grant's
    let missing = docs.docs(&["img", "missing.png"]);
    assert_eq!(
        resolve(&text(&missing), &source),
        Some(canonical(&docs.docs(&["img"])).join("missing.png"))
    );
}

#[test]
fn a_grant_refuses_escapes() {
    let docs = Docs::new();
    let source = docs.file(true);
    let mut escapes = vec![
        docs.docs(&["..", "secret.txt"]),
        docs.root().join("secret.txt"),
        docs.docs(&[]),
        docs.docs(&["img", "..", "..", "secret.txt"]),
    ];
    if cfg!(unix) {
        escapes.push(docs.docs(&["out.txt"]));
    }
    for escape in escapes {
        assert_eq!(resolve(&text(&escape), &source), None, "{escape:?}");
    }
}

#[test]
fn no_grant_serves_only_the_pages_own_file() {
    let docs = Docs::new();
    let source = docs.file(false);
    let index = docs.docs(&["index.html"]);
    assert_eq!(resolve(&text(&index), &source), Some(canonical(&index)));
    assert_eq!(resolve(&text(&docs.docs(&["img", "a.png"])), &source), None);
}

#[test]
fn a_relative_path_or_a_url_page_serves_nothing() {
    let docs = Docs::new();
    assert_eq!(resolve("docs/index.html", &docs.file(true)), None);
    let web = HtmlSource::Url(url::Url::parse("https://example.com/").unwrap());
    assert_eq!(resolve(&text(&docs.docs(&["index.html"])), &web), None);
}

/// A grant on a share (this machine's administrative share of the fixture, by its name in capitals, which
/// canonicalizing keeps) serves a request that spells the share otherwise (its server and name lowercased, or
/// verbatim), and refuses one on another volume.
#[test]
#[cfg(windows)]
fn a_grant_on_a_share_serves_its_files_in_any_spelling_of_the_share() {
    let docs = Docs::new();
    let canonical_docs = canonical(&docs.docs(&[]));
    let plain = text(&canonical_docs).strip_prefix(r"\\?\").unwrap().to_owned();
    let (drive, rest) = plain.split_once(r":\").unwrap();
    let computer = std::env::var("COMPUTERNAME").unwrap().to_uppercase();
    let grant = format!(r"\\{computer}\{drive}$\{rest}");
    let source = HtmlSource::File {
        path: format!(r"{grant}\index.html"),
        grant_root: Some(grant.clone()),
    };
    let lower = format!(r"\\{}\{}$\{rest}", computer.to_lowercase(), drive.to_lowercase());
    let verbatim = format!(r"\\?\UNC\{}\{}$\{rest}", computer.to_lowercase(), drive.to_lowercase());
    // the real paths keep each spelling, so `Path::starts_with` tells them apart
    assert!(
        !canonical(Path::new(&format!(r"{lower}\index.html"))).starts_with(canonical(Path::new(&grant))),
        "{lower} {grant}"
    );
    for base in [&grant, &lower, &verbatim] {
        let index = format!(r"{base}\index.html");
        assert_eq!(resolve(&index, &source), Some(canonical(Path::new(&index))), "{base}");
        assert_eq!(resolve(&format!(r"{base}\..\secret.txt"), &source), None, "{base}");
    }
    let elsewhere = format!(r"\\ngterm-nowhere.invalid\{drive}$\{rest}\index.html");
    assert_eq!(resolve(&elsewhere, &source), None);
    assert_eq!(resolve(&format!(r"{plain}\index.html"), &source), None);
}

/// Upstream's `testAdmissionRefusesPastItsLimitUntilASlotIsReleased`.
#[test]
fn admission_refuses_past_its_limit_until_a_slot_is_released() {
    let admission = Admission::new(2);
    assert!(admission.try_acquire());
    assert!(admission.try_acquire());
    assert!(!admission.try_acquire());
    admission.release();
    assert!(admission.try_acquire());
}

#[test]
fn the_mime_type_follows_the_extension() {
    assert_eq!(mime_type(Path::new("/d/index.html")), "text/html");
    assert_eq!(mime_type(Path::new("/d/a.png")), "image/png");
    assert_eq!(mime_type(Path::new("/d/blob")), "application/octet-stream");
}
