//! Upstream's `LinkPolicyTests.swift` text page, line location and click intent cases. The paths are Unix-spelled, so
//! those cases run where such a path is absolute; Windows has its own drive-colon case.

use std::path::PathBuf;

use uuid::Uuid;

use super::super::{LinkDisposition, disposition_for_hosts_and_home, route_for_hosts};
use super::*;

fn hosts() -> HashSet<String> {
    ["myhost", "localhost"].into_iter().map(str::to_owned).collect()
}

fn loc(path: &str, line: Option<i64>) -> LinkLocation {
    LinkLocation {
        path: path.to_owned(),
        line,
    }
}

fn session() -> Uuid {
    Uuid::from_u128(7)
}

#[cfg(unix)]
fn locations_of(raw: &str, cwd: Option<&str>) -> Vec<LinkLocation> {
    locations_for_hosts_and_home(raw, ClickOrigin::Pane(session()), cwd, &hosts(), "/Users/me")
}

#[cfg(unix)]
#[test]
fn eligible_local_files_are_text_pages() {
    for raw in [
        "/tmp/a.go",
        "/tmp/notes.txt",
        "file:///tmp/Makefile",
        "file://localhost/tmp/.zshrc",
        "~/a.swift",
    ] {
        assert!(
            matches!(
                disposition_for_hosts_and_home(raw, &hosts(), "/Users/me"),
                LinkDisposition::Text { line: None, .. }
            ),
            "{raw}"
        );
    }
}

// regression (upstream): a decoded NUL made the open read the name before it while eligibility judged the suffix
#[cfg(unix)]
#[test]
fn a_file_link_decoding_to_a_control_character_is_ignored_and_offers_nothing() {
    for raw in [
        "file:///tmp/id_rsa%00.go",
        "file:///tmp/id_rsa%00.md",
        "file://localhost/tmp/a%0A.txt",
        "file:///tmp/a%00.go:3",
    ] {
        assert_eq!(
            disposition_for_hosts_and_home(raw, &hosts(), "/Users/me"),
            LinkDisposition::Ignore,
            "{raw}"
        );
        assert!(locations_of(raw, Some("/w")).is_empty(), "{raw}");
    }
}

// regression (upstream): an absolute compiler location read its suffix as the extension and was revealed
#[cfg(unix)]
#[test]
fn an_absolute_location_names_the_literal_path_then_the_file_at_its_line() {
    assert_eq!(
        locations_of("/x/a.go:42:7", None),
        [loc("/x/a.go:42:7", None), loc("/x/a.go", Some(42))]
    );
    assert_eq!(
        locations_of("/x/a.go:42", None),
        [loc("/x/a.go:42", None), loc("/x/a.go", Some(42))]
    );
    assert_eq!(
        locations_of("file:///x/a.go:9", None),
        [loc("/x/a.go:9", None), loc("/x/a.go", Some(9))]
    );
    assert_eq!(
        locations_of("~/a.go:5", None),
        [loc("/Users/me/a.go:5", None), loc("/Users/me/a.go", Some(5))]
    );
}

// regression (upstream): a relative compiler location was parsed as a URL scheme and ignored
#[cfg(unix)]
#[test]
fn a_relative_location_joins_the_pane_directory_before_any_scheme_parse() {
    assert_eq!(
        locations_of("./a.go:42", Some("/w")),
        [loc("/w/a.go:42", None), loc("/w/a.go", Some(42))]
    );
    assert_eq!(
        locations_of("src/a.go:3:1", Some("/w/")),
        [loc("/w/src/a.go:3:1", None), loc("/w/src/a.go", Some(3))]
    );
    assert_eq!(
        locations_of("../lib/b.rs:10", Some("/w/app")),
        [loc("/w/lib/b.rs:10", None), loc("/w/lib/b.rs", Some(10))]
    );
    assert_eq!(locations_of("src/a.go", Some("/w")), [loc("/w/src/a.go", None)]);
    // this build's own: a bare name is joined as Ghostty's `resolvePathForOpening` joins it
    assert_eq!(locations_of("notes.txt", Some("/w")), [loc("/w/notes.txt", None)]);
    assert!(locations_of("notes.txt:3", Some("/w")).is_empty());
}

#[cfg(unix)]
#[test]
fn an_absolute_path_without_a_valid_line_needs_no_lookup() {
    for raw in [
        "/x/a.go",
        "/x/a.go:",
        "/x/a.go:0",
        "/x/a.go:-3",
        "/x/a.go:abc",
        "/x/a.go:4x",
        "/x/a.go:99999999999999999999",
        "/x/:12",
    ] {
        assert!(locations_of(raw, None).is_empty(), "{raw}");
    }
}

#[cfg(unix)]
#[test]
fn web_and_foreign_links_are_never_paths() {
    for raw in [
        "http://localhost:3000",
        "https://host:8443",
        "https://host:8443/a.go:3",
        "mailto:a@b.c",
        "x-custom://run/a.go:3",
        "file://otherhost/x/a.go:3",
        "file:////server/share/a.go:3",
    ] {
        assert!(locations_of(raw, Some("/w")).is_empty(), "{raw}");
    }
}

#[cfg(unix)]
#[test]
fn refused_absolute_paths_offer_nothing_to_stat() {
    let mut refused = vec![
        "/net/x/a.go:3",
        "/tmp/../net/x/a.go:3",
        "//host/share/a.go:3",
        "/misc/a.go:3",
    ];
    if cfg!(target_os = "macos") {
        refused.extend([
            "/Network/a.go:3",
            "/home/me/a.go:3",
            "/System/Volumes/Data/home/me/a.go:3",
        ]);
    }
    for raw in refused {
        assert!(locations_of(raw, Some("/w")).is_empty(), "{raw}");
    }
}

#[cfg(unix)]
#[test]
fn a_relative_path_under_a_missing_or_refused_directory_offers_nothing() {
    for cwd in [
        None,
        Some(""),
        Some("rel/dir"),
        Some("/net/x"),
        Some("/w\u{07}"),
        Some("/w\nx"),
        Some("//host/share"),
    ] {
        assert!(locations_of("src/a.go:3", cwd).is_empty(), "{cwd:?}");
        assert!(locations_of("./a.go:3", cwd).is_empty(), "{cwd:?}");
    }
}

#[cfg(unix)]
#[test]
fn a_relative_path_escaping_into_an_automount_root_offers_nothing() {
    assert!(locations_of("../../net/x/a.go:3", Some("/w/app")).is_empty());
    assert!(locations_of("../../../misc/x/a.go:3", Some("/w/a/b")).is_empty());
}

#[cfg(unix)]
#[test]
fn a_remote_pane_offers_nothing_to_stat() {
    let remote = ClickOrigin::RemotePane(session());
    for (raw, cwd) in [
        ("/x/a.go:42", None),
        ("./a.go:42", Some("/w")),
        ("file:///x/a.go:42", None),
    ] {
        assert!(locations_for_hosts_and_home(raw, remote, cwd, &hosts(), "/Users/me").is_empty());
    }
}

#[cfg(unix)]
#[test]
fn no_found_candidate_keeps_todays_route() {
    let route = |raw: &str| route_for_hosts(raw, LinkOpenMode::Overlay, ClickOrigin::Pane(session()), &hosts());
    assert_eq!(route("/x/a.go:42"), Route::Reveal(PathBuf::from("/x/a.go:42")));
    assert_eq!(route("./a.go:42"), Route::Ignore);
    assert_eq!(route("src/a.go:42"), Route::Ignore);
}

#[cfg(unix)]
#[test]
fn a_found_location_opens_as_text_at_its_line_from_a_pane_or_the_scratch() {
    let session = session();
    for mode in LinkOpenMode::ALL {
        let size = if *mode == LinkOpenMode::Browser {
            Some(95)
        } else {
            mode.overlay_size_percent()
        };
        for origin in [ClickOrigin::Pane(session), ClickOrigin::Scratch(session)] {
            assert_eq!(
                route_for_location(&loc("/x/a.go", Some(42)), *mode, origin),
                Route::Text {
                    path: PathBuf::from("/x/a.go"),
                    line: Some(42),
                    session,
                    size_percent: size
                }
            );
        }
        assert_eq!(
            route_for_hosts("/x/a.go", *mode, ClickOrigin::Pane(session), &hosts()),
            Route::Text {
                path: PathBuf::from("/x/a.go"),
                line: None,
                session,
                size_percent: size
            }
        );
    }
}

#[cfg(unix)]
#[test]
fn a_text_file_from_another_origin_is_revealed() {
    let file = PathBuf::from("/x/a.go");
    for origin in [
        ClickOrigin::RemotePane(session()),
        ClickOrigin::Hud,
        ClickOrigin::ProgramOverlay,
        ClickOrigin::Quick,
    ] {
        assert_eq!(
            route_for_location(&loc("/x/a.go", Some(42)), LinkOpenMode::Overlay, origin),
            Route::Reveal(file.clone())
        );
        assert_eq!(
            route_for_hosts("file:///x/a.go", LinkOpenMode::Overlay, origin, &hosts()),
            Route::Reveal(file.clone())
        );
    }
}

#[cfg(unix)]
#[test]
fn a_found_location_keeps_the_image_and_markdown_routes_and_reveals_the_rest() {
    let session = session();
    let pane = ClickOrigin::Pane(session);
    assert_eq!(
        route_for_location(&loc("/x/a.png", Some(3)), LinkOpenMode::Overlay, pane),
        Route::Image {
            path: PathBuf::from("/x/a.png"),
            session
        }
    );
    assert_eq!(
        route_for_location(&loc("/x/r.md", Some(3)), LinkOpenMode::Overlay, pane),
        Route::Markdown {
            path: PathBuf::from("/x/r.md"),
            session,
            size_percent: None
        }
    );
    assert_eq!(
        route_for_location(&loc("/x/a.go:42", None), LinkOpenMode::Overlay, pane),
        Route::Reveal(PathBuf::from("/x/a.go:42"))
    );
}

#[cfg(windows)]
#[test]
fn a_windows_drive_colon_stays_part_of_the_path() {
    let found = locations_for_hosts_and_home(
        "C:\\x\\a.go:42",
        ClickOrigin::Pane(session()),
        None,
        &hosts(),
        "C:\\Users\\me",
    );
    assert_eq!(found, [loc("C:\\x\\a.go:42", None), loc("C:\\x\\a.go", Some(42))]);
    let relative = locations_for_hosts_and_home(
        "src\\a.go:3",
        ClickOrigin::Pane(session()),
        Some("C:\\w"),
        &hosts(),
        "C:\\Users\\me",
    );
    assert_eq!(
        relative,
        [loc("C:\\w\\src\\a.go:3", None), loc("C:\\w\\src\\a.go", Some(3))]
    );
}

/// Upstream's `theChordDecidesTheClickIntent`, Ctrl or Super standing for ⌘ and Alt for ⌥.
#[test]
fn the_chord_decides_the_click_intent() {
    let (alt, shift) = (KeyModifiers::OPTION, KeyModifiers::SHIFT);
    for command in [KeyModifiers::CONTROL, KeyModifiers::COMMAND] {
        let other = if command == KeyModifiers::CONTROL {
            KeyModifiers::COMMAND
        } else {
            KeyModifiers::CONTROL
        };
        for (modifiers, captured, intent) in [
            (command, false, ClickIntent::Normal),
            (command | shift, false, ClickIntent::Browser),
            (command | alt, false, ClickIntent::Reveal),
            (command | alt | shift, false, ClickIntent::Normal),
            (alt, false, ClickIntent::Normal),
            (command | other, false, ClickIntent::Normal),
            (command | alt | other, false, ClickIntent::Normal),
            (command, true, ClickIntent::Normal),
            (command | shift, true, ClickIntent::Normal),
            (command | alt, true, ClickIntent::Normal),
            (command | alt | shift, true, ClickIntent::Reveal),
        ] {
            let pane = ClickOrigin::Pane(session());
            assert_eq!(
                click_intent(modifiers, LinkOpenMode::Overlay, pane, captured),
                intent,
                "{modifiers:?} {captured}"
            );
        }
    }
}

/// Upstream's `theRevealChordWorksInEveryModeAndOrigin`.
#[test]
fn the_reveal_chord_works_in_every_mode_and_origin() {
    let session = session();
    let reveal = KeyModifiers::CONTROL | KeyModifiers::OPTION;
    for mode in LinkOpenMode::ALL {
        for origin in [
            ClickOrigin::Pane(session),
            ClickOrigin::RemotePane(session),
            ClickOrigin::Scratch(session),
            ClickOrigin::Hud,
            ClickOrigin::ProgramOverlay,
            ClickOrigin::Quick,
        ] {
            assert_eq!(click_intent(reveal, *mode, origin, false), ClickIntent::Reveal);
            assert_eq!(
                click_intent(reveal | KeyModifiers::SHIFT, *mode, origin, true),
                ClickIntent::Reveal
            );
        }
    }
}

/// Upstream's `aLocalRouteNamesItsFileAndAWebOrIgnoredOneNone`.
#[test]
fn a_local_route_names_its_file_and_a_web_or_ignored_one_none() {
    let file = PathBuf::from("/x/a.go");
    let session = session();
    let local = [
        Route::Image {
            path: file.clone(),
            session,
        },
        Route::Markdown {
            path: file.clone(),
            session,
            size_percent: Some(95),
        },
        Route::Text {
            path: file.clone(),
            line: Some(3),
            session,
            size_percent: None,
        },
        Route::Reveal(file.clone()),
    ];
    for route in local {
        assert_eq!(route.local_file(), Some(file.as_path()));
    }
    let web = "https://example.com/".to_owned();
    assert_eq!(Route::Browser(web.clone()).local_file(), None);
    let overlay = Route::Overlay {
        url: web,
        session,
        size_percent: None,
    };
    assert_eq!(overlay.local_file(), None);
    assert_eq!(Route::Ignore.local_file(), None);
}
