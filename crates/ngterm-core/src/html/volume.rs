//! A path's volume as every grant check reads it: a page's grant names a directory, and a read, a navigation or a
//! resource request names a file, each spelled by its own source. On Windows a drive or a share has several
//! spellings that name the same volume: verbatim (`\\?\C:`, `\\?\UNC\server\share`, what canonicalizing gives) or
//! plain, a drive's letter or a share's name in any case, and a share's server as the user typed it or as a page's
//! `file:` address carries it (lowercased, a non-ASCII name punycoded). [`volume`] gives each one spelling, so the
//! grant's checks ([`super::contains`], [`super::grant_error`], the frontends' grant schemes, a replica's resource
//! requests) all compare volumes alike. A Unix path has no volume.

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
                let server = server.to_string_lossy();
                let server = url::Host::parse(&server).map_or_else(|_| server.to_string(), |host| host.to_string());
                format!(r"\\{server}\{}", share.to_string_lossy())
            }
            Prefix::Verbatim(_) | Prefix::DeviceNS(_) => prefix.as_os_str().to_string_lossy().into_owned(),
        }
        .to_lowercase(),
    )
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
        // the rest of the path compares exactly: canonicalizing gives each name its spelling on disk
        assert!(!strictly_inside(Path::new(r"\\?\C:\Site\a"), Path::new(r"\\?\C:\site")));
    }
}
