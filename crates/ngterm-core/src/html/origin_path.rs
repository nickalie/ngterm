//! Lexical paths on another machine, in that machine's own conventions. A viewer lists the pictures of a Markdown file
//! another machine holds against its folder there (upstream's `remoteImageReferences`), and that machine may run
//! another OS than this one: its path's own shape, not this OS's [`std::path`] rules, says how it splits and joins. A
//! `/`-rooted path is POSIX (`/` alone separates, names compare exactly); a path rooted on a drive (`C:\`, `C:/`), a
//! share (`\\server\share\`) or either behind the verbatim `\\?\` prefix is Windows (`\` and `/` both separate, names
//! compare in any case). Anything else is not absolute anywhere, so nothing lies inside it.
//!
//! No filesystem is touched: the machine holding the files resolves each candidate, symlinks included, before it
//! serves one (`crate::remote::page_serve::content`).

/// An absolute folder on the machine that holds it: its root (`/`, `C:\`, `\\server\share\`), then its names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OriginFolder {
    windows: bool,
    root: String,
    names: Vec<String>,
}

impl OriginFolder {
    /// The folder holding the file at `file`, an absolute path on its own machine; `None` for a path that is absolute
    /// on no machine, or a root alone.
    pub(super) fn of_file(file: &str) -> Option<Self> {
        let mut folder = Self::absolute(file)?;
        folder.names.pop()?;
        Some(folder)
    }

    /// `path` as an absolute path of the shape it has, dot-normalized (`..` never climbs above its root).
    fn absolute(path: &str) -> Option<Self> {
        if let Some(rest) = path.strip_prefix('/') {
            return Some(Self::rooted(false, "/".to_owned(), rest));
        }
        let (root, rest) = windows_root(path)?;
        Some(Self::rooted(true, root, rest))
    }

    fn rooted(windows: bool, root: String, rest: &str) -> Self {
        let folder = Self {
            windows,
            root,
            names: Vec::new(),
        };
        folder.joined(rest)
    }

    fn joined(mut self, relative: &str) -> Self {
        let windows = self.windows;
        for part in relative.split(|ch| ch == '/' || (windows && ch == '\\')) {
            match part {
                "" | "." => {}
                ".." => {
                    self.names.pop();
                }
                name => self.names.push(name.to_owned()),
            }
        }
        self
    }

    /// `reference` joined to this folder and normalized as its own machine would (upstream's `lexicalPath`), spelled in
    /// that machine's separators: `None` unless the result lies strictly inside the folder, so no `~`, no scheme and
    /// no `..` escape. On a Windows folder a `\`- or `/`-rooted reference lands on the folder's own drive, as it would
    /// there.
    pub(super) fn lexical_path(&self, reference: &str) -> Option<String> {
        if reference.is_empty() || reference.starts_with('~') {
            return None;
        }
        let candidate = match Self::absolute(reference) {
            Some(absolute) if absolute.windows == self.windows => absolute,
            // a drive's path is a scheme on a POSIX machine, as `C:name` is on a Windows one
            _ if super::markdown_page::has_scheme(reference) => return None,
            // on Windows a `\`- or `/`-rooted path is on the current drive; on POSIX a share's spelling is a name
            _ if self.windows && reference.starts_with(['\\', '/']) => Self::rooted(true, self.root.clone(), reference),
            _ => self.clone().joined(reference),
        };
        (candidate.names.len() > self.names.len() && self.holds(&candidate)).then(|| candidate.spelled())
    }

    /// Whether `other` starts with this folder's root and names, compared as its machine compares them.
    fn holds(&self, other: &Self) -> bool {
        let same = |a: &str, b: &str| {
            if self.windows {
                a.to_lowercase() == b.to_lowercase()
            } else {
                a == b
            }
        };
        same(&self.root, &other.root) && self.names.iter().zip(&other.names).all(|(a, b)| same(a, b))
    }

    fn spelled(&self) -> String {
        let separator = if self.windows { "\\" } else { "/" };
        format!("{}{}", self.root, self.names.join(separator))
    }
}

/// The root of a Windows absolute path, spelled with `\`, and the rest after it; `None` for any other path.
fn windows_root(path: &str) -> Option<(String, &str)> {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        let (share, rest) = share(rest)?;
        return Some((format!(r"\\?\UNC\{share}\"), rest));
    }
    if let Some(rest) = path.strip_prefix(r"\\?\") {
        let (drive, rest) = drive(rest)?;
        return Some((format!(r"\\?\{drive}\"), rest));
    }
    if let Some(rest) = path.strip_prefix(r"\\") {
        let (share, rest) = share(rest)?;
        return Some((format!(r"\\{share}\"), rest));
    }
    let (drive, rest) = drive(path)?;
    Some((format!(r"{drive}\"), rest))
}

/// A drive and its root (`C:\` or `C:/`, never a drive-relative `C:name`): the drive, then what follows its root.
fn drive(path: &str) -> Option<(&str, &str)> {
    let mut chars = path.chars();
    let letter = chars.next()?;
    (letter.is_ascii_alphabetic() && chars.next() == Some(':') && matches!(chars.next(), Some('\\' | '/')))
        .then(|| (&path[..2], &path[3..]))
}

/// A share's server and name, spelled `server\share`, then what follows them.
fn share(path: &str) -> Option<(String, &str)> {
    let mut parts = path.splitn(3, ['\\', '/']);
    let server = parts.next().filter(|name| !name.is_empty())?;
    let name = parts.next().filter(|name| !name.is_empty())?;
    Some((format!(r"{server}\{name}"), parts.next().unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::OriginFolder;

    fn folder(file: &str) -> OriginFolder {
        OriginFolder::of_file(file).unwrap_or_else(|| panic!("{file} has a folder"))
    }

    /// A POSIX origin's folder joins and normalizes with `/` alone on any host: a backslash is part of a name, a
    /// Windows-shaped reference is a scheme or stays a name, and nothing leaves the folder.
    #[test]
    fn a_posix_origin_joins_with_its_own_rules_on_any_host() {
        let docs = folder("/srv/docs/README.md");
        for (reference, expected) in [
            ("a.png", "/srv/docs/a.png"),
            ("./img/../b.png", "/srv/docs/b.png"),
            ("img//c.png", "/srv/docs/img/c.png"),
            ("/srv/docs/d.png", "/srv/docs/d.png"),
            (r"img\e.png", r"/srv/docs/img\e.png"),
            (r"\\server\share\a.png", r"/srv/docs/\\server\share\a.png"),
        ] {
            assert_eq!(docs.lexical_path(reference).as_deref(), Some(expected), "{reference}");
        }
        for reference in [
            "../a.png",
            "~/a.png",
            "https://x.io/a.png",
            "",
            ".",
            "img/..",
            "/etc/hosts",
            "/srv/docsx/a.png",
            "/srv/Docs/a.png",
            r"C:\srv\docs\a.png",
            "C:/srv/docs/a.png",
        ] {
            assert_eq!(docs.lexical_path(reference), None, "{reference}");
        }
        assert_eq!(folder("/README.md").lexical_path("a.png").as_deref(), Some("/a.png"));
    }

    /// A Windows origin's folder joins with `\` and `/` alike on any host, compares names in any case, keeps a rooted
    /// reference on its own drive and spells the result with `\`.
    #[test]
    fn a_windows_origin_joins_with_its_own_rules_on_any_host() {
        let docs = folder(r"C:\Users\me\docs\README.md");
        for (reference, expected) in [
            ("a.png", r"C:\Users\me\docs\a.png"),
            ("img/../b.png", r"C:\Users\me\docs\b.png"),
            (r"img\c.png", r"C:\Users\me\docs\img\c.png"),
            (r"c:\users\ME\docs\d.png", r"c:\users\ME\docs\d.png"),
            (r"\Users\me\docs\e.png", r"C:\Users\me\docs\e.png"),
            ("C:/Users/me/docs/f.png", r"C:\Users\me\docs\f.png"),
        ] {
            assert_eq!(docs.lexical_path(reference).as_deref(), Some(expected), "{reference}");
        }
        for reference in [
            "../a.png",
            r"..\a.png",
            "~/a.png",
            "https://x.io/a.png",
            "C:a.png",
            r"D:\Users\me\docs\a.png",
            r"\Windows\win.ini",
            "/etc/hosts",
            "",
            ".",
        ] {
            assert_eq!(docs.lexical_path(reference), None, "{reference}");
        }
        assert_eq!(
            folder("C:/docs/README.md").lexical_path("a.png").as_deref(),
            Some(r"C:\docs\a.png")
        );
        assert_eq!(
            folder(r"C:\README.md").lexical_path("a.png").as_deref(),
            Some(r"C:\a.png")
        );
    }

    /// A share and the verbatim prefix are Windows roots too; `..` stops at a root.
    #[test]
    fn shares_and_verbatim_paths_are_windows_roots() {
        let share = folder(r"\\server\share\docs\README.md");
        assert_eq!(
            share.lexical_path("a.png").as_deref(),
            Some(r"\\server\share\docs\a.png")
        );
        assert_eq!(share.lexical_path(r"\a.png"), None);
        assert_eq!(
            folder(r"\\?\C:\docs\README.md").lexical_path("a.png").as_deref(),
            Some(r"\\?\C:\docs\a.png")
        );
        assert_eq!(
            folder(r"\\?\UNC\server\share\README.md")
                .lexical_path("a.png")
                .as_deref(),
            Some(r"\\?\UNC\server\share\a.png")
        );
        assert_eq!(
            folder(r"C:\..\..\docs\README.md").lexical_path("a.png").as_deref(),
            Some(r"C:\docs\a.png")
        );
    }

    /// A path absolute on no machine, a drive-relative one and a root alone have no folder.
    #[test]
    fn a_path_absolute_nowhere_has_no_folder() {
        for file in [
            "README.md",
            "docs/README.md",
            "C:README.md",
            r"\\server",
            r"\\server\",
            "/",
            r"C:\",
            "",
        ] {
            assert_eq!(OriginFolder::of_file(file), None, "{file}");
        }
    }
}
