//! A path's volume as every grant check reads it: a page's grant names a directory, and a read, a navigation or a
//! resource request names a file, each spelled by its own source. On Windows a drive or a share has several
//! spellings that name the same volume: verbatim (`\\?\C:`, `\\?\UNC\server\share`, what canonicalizing gives) or
//! plain, a drive's letter or a share's name in any case, and a share's server as the user typed it or as a page's
//! `file:` address carries it (lowercased, a non-ASCII name punycoded). [`volume`] gives each one spelling, so the
//! grant's checks ([`super::contains`], [`super::grant_error`], the frontends' grant schemes, a replica's resource
//! requests) all compare volumes alike. A Unix path has no volume.
//!
//! Two spellings are one only when Windows reaches the same host through both ([`server`]): a check that equated two
//! servers would let a request on the other one through to the file system, and Windows' SMB connection to a server
//! a page chose hands over the user's NTLM credentials.

use std::path::{Component, Path, Prefix};

/// The volume `path` names, in one spelling whether verbatim (`\\?\`) or not and in any case, as Windows reads it: a
/// drive's letter, or a share's server and name, the server as a `file:` URI's host spells it (lowercased, a
/// non-ASCII name punycoded, as a page's address carries it); any other prefix (a device, a volume GUID) as written,
/// lowercased; `None` for a path without one (every Unix path, a Windows path rooted on the current drive).
#[must_use]
pub fn volume(path: &Path) -> Option<String> {
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return None;
    };
    Some(
        match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => format!("{}:", char::from(letter)),
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                format!(
                    r"\\{}\{}",
                    self::server(&server.to_string_lossy()),
                    share.to_string_lossy()
                )
            }
            Prefix::Verbatim(_) | Prefix::DeviceNS(_) => prefix.as_os_str().to_string_lossy().into_owned(),
        }
        .to_lowercase(),
    )
}

/// A share's server as [`volume`] spells it: as a `file:` URI's host carries it (lowercased, a non-ASCII name
/// punycoded, the loopback name `localhost`, which a `file:` URI drops, as its address `127.0.0.1`) when that is only
/// the name in another case or its IDNA form, else as written, lowercased. The URL standard's host parser also
/// percent-decodes, maps compatibility forms (fullwidth letters) and reads `127.1` as an address; Windows reads each
/// as a name of its own, so `n%61s` keeps a spelling no `nas` has. A name written as itself in any spelling compares
/// equal: the URL spelling is its own host's.
#[must_use]
pub fn server(name: &str) -> String {
    let lower = name.to_lowercase();
    let Ok(host) = url::Host::parse(name) else {
        return lower;
    };
    let spelled = host.to_string();
    let same = spelled == lower || {
        let (unicode, errors) = idna::domain_to_unicode(&spelled);
        errors.is_ok() && unicode == lower
    };
    match spelled.as_str() {
        _ if !same => lower,
        "localhost" => "127.0.0.1".to_owned(),
        _ => spelled,
    }
}

/// Whether `path` sits strictly inside `root`: on the same [`volume`], and below it by the components after the
/// volume, which compare exactly (real paths, whose names canonicalizing spells as on disk). `Path::starts_with`
/// compares a verbatim and a plain prefix as different, and a share's server exactly.
#[must_use]
pub fn strictly_inside(path: &Path, root: &Path) -> bool {
    fn after_volume(path: &Path) -> Vec<Component<'_>> {
        path.components()
            .skip_while(|component| matches!(component, Component::Prefix(_)))
            .collect()
    }
    let (path_rest, root_rest) = (after_volume(path), after_volume(root));
    volume(path) == volume(root) && path_rest.len() > root_rest.len() && path_rest.starts_with(&root_rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_strictly_inside_its_roots_descendants_only() {
        let root = Path::new("/srv/site");
        assert!(strictly_inside(Path::new("/srv/site/index.html"), root));
        assert!(strictly_inside(Path::new("/srv/site/a/b.png"), root));
        assert!(!strictly_inside(root, root));
        assert!(!strictly_inside(Path::new("/srv/site/"), root));
        assert!(!strictly_inside(Path::new("/srv/sites/index.html"), root));
        assert!(!strictly_inside(Path::new("/srv/index.html"), root));
        assert_eq!(volume(Path::new("/srv/site")), None);
    }

    /// A server compares in another case, in its IDNA form, and `localhost` as its loopback address; a spelling the
    /// URL parser would read as another name (percent-escaped, a compatibility form, a shorthand address) stays apart.
    #[test]
    fn a_server_is_spelled_as_its_address_only_when_windows_reads_the_same_host() {
        assert_eq!(server("NAS"), "nas");
        assert_eq!(server("ÑAS"), "xn--as-yja");
        assert_eq!(server("xn--as-yja"), "xn--as-yja");
        assert_eq!(server("LocalHost"), "127.0.0.1");
        assert_eq!(server("127.0.0.1"), "127.0.0.1");
        assert_eq!(server("Host.Example"), "host.example");
        for (spelling, name) in [
            ("n%61s", "nas"),
            ("N%61S", "nas"),
            ("ｎａｓ", "nas"),
            ("127.1", "127.0.0.1"),
            ("0x7f.0.0.1", "127.0.0.1"),
            ("local%68ost", "127.0.0.1"),
            ("ngterm-nowhere%2Einvalid", "ngterm-nowhere.invalid"),
        ] {
            assert_ne!(server(spelling), server(name), "{spelling}");
            assert_eq!(server(spelling), server(&spelling.to_uppercase()), "{spelling}");
        }
    }

    #[test]
    #[cfg(windows)]
    fn a_volume_is_a_drive_or_a_share_in_any_spelling() {
        let volume = |path: &str| volume(Path::new(path));
        assert_eq!(volume(r"C:\a\b"), volume(r"\\?\c:\x"));
        assert_ne!(volume(r"C:\a"), volume(r"D:\a"));
        assert_eq!(volume(r"\\Server\Share\a"), volume(r"\\?\UNC\server\share\b"));
        assert_ne!(volume(r"\\server\share\a"), volume(r"\\server\other\a"));
        assert_ne!(volume(r"\\server\share\a"), volume(r"\\elsewhere\share\a"));
        assert_ne!(volume(r"C:\a"), volume(r"\\localhost\C$\a"));
        assert_eq!(volume(r"\a\b"), None);
        // a server as a page's address spells it: lowercased, punycoded
        assert_eq!(volume(r"\\ÑAS\share\a"), volume(r"\\xn--as-yja\share\a"));
        assert_eq!(volume(r"\\?\UNC\ÑAS\Share\a"), volume(r"\\xn--as-yja\share\a"));
        assert_eq!(volume(r"\\localhost\C$\a"), volume(r"\\127.0.0.1\c$\a"));
        // a server Windows reads as another name stays apart
        assert_ne!(volume(r"\\n%61s\share\a"), volume(r"\\nas\share\a"));
        assert_ne!(volume(r"\\?\UNC\n%61s\share\a"), volume(r"\\nas\share\a"));
    }

    /// A share's server spelled differently by the grant and by a page's address, and a verbatim path against a plain
    /// one (`Path::starts_with` refuses both).
    #[test]
    #[cfg(windows)]
    fn a_file_is_inside_its_grant_whatever_its_volumes_spelling() {
        let root = Path::new(r"\\?\UNC\NAS\Share\site");
        let page = Path::new(r"\\?\UNC\nas\share\site\index.html");
        assert!(!page.starts_with(root));
        assert!(strictly_inside(page, root));
        assert!(strictly_inside(
            Path::new(r"\\?\UNC\xn--as-yja\share\site\a"),
            Path::new(r"\\?\UNC\ñas\share\site")
        ));
        assert!(strictly_inside(Path::new(r"\\nas\share\site\a"), root));
        assert!(strictly_inside(Path::new(r"C:\site\a"), Path::new(r"\\?\C:\site")));
        assert!(strictly_inside(Path::new(r"\\?\c:\site\a"), Path::new(r"C:\site")));
        assert!(!strictly_inside(root, root));
        assert!(!strictly_inside(Path::new(r"\\nas\share\site"), root));
        assert!(!strictly_inside(Path::new(r"\\?\UNC\nas\share\other\a"), root));
        assert!(!strictly_inside(Path::new(r"\\?\UNC\nas\other\site\a"), root));
        assert!(!strictly_inside(Path::new(r"\\?\C:\site\a"), Path::new(r"\\?\D:\site")));
        assert!(!strictly_inside(
            Path::new(r"\\n%61s\share\site\a"),
            Path::new(r"\\nas\share\site")
        ));
        assert!(strictly_inside(
            Path::new(r"\\127.0.0.1\C$\site\a"),
            Path::new(r"\\?\UNC\localhost\c$\site")
        ));
        // the rest of the path compares exactly: canonicalizing gives each name its spelling on disk
        assert!(!strictly_inside(Path::new(r"\\?\C:\Site\a"), Path::new(r"\\?\C:\site")));
    }
}
