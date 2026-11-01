//! What one pane of Duplicate as Companion starts (upstream's `DuplicateReplay.swift`): another copy of the program
//! holding the source pane, in that program's directory, or a plain shell where a shell holds it. A pipeline or a
//! shell job is copied as its observed foreground process only.
//!
//! A program not started by an absolute path is copied by the path it was started from, because the creation command
//! runs on the app's `PATH`, not the source shell's: for a bare name the first entry of the source's own `PATH` that
//! resolves to the running image, else the first executable one (the shell's own pick when a version manager's shim
//! re-exec'd); for a relative name its directory. It launches unresolved, so `vimdiff` stays `vimdiff`. With no `PATH`
//! to read (macOS withholds a platform binary's environment) the system directories are searched and the image is the
//! fallback; otherwise one that cannot be located is refused rather than copied as the image. A bare name whose image
//! is gone (an upgrade removed it) starts what the pane's `PATH` names now. The restart replay's checks
//! ([`restart_replay::resolve`]) see both the starting path and the image.
//!
//! [`PathRules`] carries the lookup per OS: POSIX `dir/name` over the absolute `:` entries; Windows `;` entries rooted
//! at a drive or a UNC share, each tried as named (when it has an extension) and with each `PATHEXT` extension, names
//! compared in any case, and `%SystemRoot%\System32;%SystemRoot%` for the system directories. The backslash refusal
//! (upstream's: a POSIX creation command reaches `sh -c`, and fish's `eval` under Live, which read a backslash inside
//! single quotes differently) applies to the POSIX rules only, an ngterm adaptation: a backslash separates a Windows
//! path, whose command line is quoted in its own dialect.

use std::path::Path;

use crate::command_restore::PaneForeground;
use crate::restart_replay::{self, Observation};

/// What a duplicated pane starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// A plain login shell.
    Shell { working_directory: String },
    /// The program, as the pane's ending creation command.
    Program {
        argv: Vec<String>,
        working_directory: String,
    },
}

impl Plan {
    #[must_use]
    pub fn working_directory(&self) -> &str {
        match self {
            Self::Shell { working_directory } | Self::Program { working_directory, .. } => working_directory,
        }
    }

    /// The program to start; `None` for a plain shell.
    #[must_use]
    pub fn argv(&self) -> Option<&[String]> {
        match self {
            Self::Shell { .. } => None,
            Self::Program { argv, .. } => Some(argv),
        }
    }
}

/// Why a pane cannot be copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    Replay(restart_replay::Refusal),
    Backslash,
    NoExecutable,
    /// No path the program could have been started from resolves to the running image.
    NotLocated,
}

/// A pane that cannot be copied; the duplicate creates nothing when any pane refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub pane: String,
    pub reason: Reason,
}

impl Refusal {
    #[must_use]
    pub fn message(&self) -> String {
        let text = match &self.reason {
            Reason::Replay(refusal) => refusal.reason(),
            Reason::Backslash => {
                "the foreground program's arguments contain a backslash, which cannot be copied".to_owned()
            }
            Reason::NoExecutable => "the foreground program's executable path cannot be read".to_owned(),
            Reason::NotLocated => "the path the foreground program was started from cannot be found".to_owned(),
        };
        format!("{} pane: {text}; nothing was created", self.pane)
    }
}

/// How an OS names and looks up a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRules {
    Posix,
    Windows,
}

impl PathRules {
    /// This OS's rules.
    pub const HOST: Self = if cfg!(windows) { Self::Windows } else { Self::Posix };

    fn is_absolute(self, path: &str) -> bool {
        match self {
            Self::Posix => path.starts_with('/'),
            Self::Windows => {
                let bytes = path.as_bytes();
                path.starts_with(r"\\")
                    || path.starts_with("//")
                    || (bytes.len() >= 3
                        && bytes[0].is_ascii_alphabetic()
                        && bytes[1] == b':'
                        && matches!(bytes[2], b'\\' | b'/'))
            }
        }
    }

    fn has_separator(self, name: &str) -> bool {
        match self {
            Self::Posix => name.contains('/'),
            Self::Windows => name.contains(['/', '\\']),
        }
    }

    fn separators(self) -> &'static [char] {
        match self {
            Self::Posix => &['/'],
            Self::Windows => &['/', '\\'],
        }
    }

    fn list_separator(self) -> char {
        match self {
            Self::Posix => ':',
            Self::Windows => ';',
        }
    }

    fn same_file(self, one: &str, other: &str) -> bool {
        match self {
            Self::Posix => one == other,
            Self::Windows => one.to_lowercase() == other.to_lowercase(),
        }
    }

    /// `name` in directory `directory`.
    fn child(self, directory: &str, name: &str) -> String {
        if directory.ends_with(self.separators()) {
            format!("{directory}{name}")
        } else {
            let separator = if self == Self::Windows { '\\' } else { '/' };
            format!("{directory}{separator}{name}")
        }
    }

    /// `program` under `directory`, its `.` and `..` resolved by name, so an invocation symlink keeps its name. A
    /// Windows `program` rooted by one leading separator (`\tools\x.exe`) starts at `directory`'s drive or share root.
    fn joined(self, program: &str, directory: &str) -> String {
        let (root, rest) = match self {
            Self::Posix => ("/".to_owned(), directory),
            Self::Windows => {
                let (root, rest) = windows_root(directory);
                (
                    root,
                    if program.starts_with(self.separators()) {
                        ""
                    } else {
                        rest
                    },
                )
            }
        };
        let mut parts: Vec<&str> = Vec::new();
        for part in rest.split(self.separators()).chain(program.split(self.separators())) {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                _ => parts.push(part),
            }
        }
        match self {
            Self::Posix => root + &parts.join("/"),
            Self::Windows => format!("{root}\\{}", parts.join("\\")),
        }
    }
}

/// A Windows path's root (`C:`, `\\server\share`) and the rest.
fn windows_root(path: &str) -> (String, &str) {
    if let Some(unc) = path.strip_prefix(r"\\").or_else(|| path.strip_prefix("//")) {
        let mut parts = unc.splitn(3, ['\\', '/']);
        let server = parts.next().unwrap_or_default();
        let share = parts.next().unwrap_or_default();
        return (format!(r"\\{server}\{share}"), parts.next().unwrap_or_default());
    }
    match path.get(..2) {
        Some(drive) if drive.ends_with(':') => (drive.to_owned(), &path[2..]),
        _ => (String::new(), path),
    }
}

/// Where a bare name is looked for: the OS's rules, the system directories searched when the process's `PATH` is
/// unreadable, and Windows' `PATHEXT` extensions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lookup {
    pub rules: PathRules,
    pub system_search_path: String,
    pub extensions: Vec<String>,
}

impl Lookup {
    /// The POSIX lookup (upstream's `systemSearchPath`).
    #[must_use]
    pub fn posix() -> Self {
        Self {
            rules: PathRules::Posix,
            system_search_path: "/usr/bin:/bin:/usr/sbin:/sbin".to_owned(),
            extensions: Vec::new(),
        }
    }

    /// The Windows lookup under `system_root` (`%SystemRoot%`) with `pathext` (`%PATHEXT%`).
    #[must_use]
    pub fn windows(system_root: &str, pathext: &str) -> Self {
        Self {
            rules: PathRules::Windows,
            system_search_path: format!(r"{system_root}\System32;{system_root}"),
            extensions: pathext
                .split(';')
                .filter(|extension| extension.starts_with('.') && extension.len() > 1)
                .map(str::to_owned)
                .collect(),
        }
    }

    /// This OS's lookup, Windows' read from the app's environment.
    #[must_use]
    pub fn host() -> Self {
        match PathRules::HOST {
            PathRules::Posix => Self::posix(),
            PathRules::Windows => {
                let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
                let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
                Self::windows(&root, &pathext)
            }
        }
    }

    /// `path` as named (Windows: when its name has an extension) and with each extension.
    fn spellings(&self, path: String) -> Vec<String> {
        if self.rules == PathRules::Posix {
            return vec![path];
        }
        let name = path.rsplit(['\\', '/']).next().unwrap_or(&path);
        let mut spellings: Vec<String> = Vec::new();
        if name.contains('.') {
            spellings.push(path.clone());
        }
        spellings.extend(self.extensions.iter().map(|extension| format!("{path}{extension}")));
        spellings
    }
}

/// `path` with every symlink resolved when it names an executable file, `None` otherwise (upstream's
/// `canonicalExecutable`).
#[must_use]
pub fn canonical_executable(path: &str) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || !is_executable(&metadata) {
        return None;
    }
    let canonical = std::fs::canonicalize(Path::new(path)).ok()?;
    let canonical = canonical.into_os_string().into_string().ok()?;
    Some(without_verbatim_prefix(canonical))
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    true
}

/// A Windows canonical path without its `\\?\` prefix, as a process's image path is spelled.
fn without_verbatim_prefix(path: String) -> String {
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    if let Some(local) = path.strip_prefix(r"\\?\") {
        return local.to_owned();
    }
    path
}

/// Plans the panes of one duplicate.
pub struct Planner<'a> {
    /// The user's `$SHELL` basename.
    pub shell: Option<&'a str>,
    pub denylist: &'a [String],
    pub lookup: &'a Lookup,
    /// The file a path resolves to when it names an executable ([`canonical_executable`]); a seam for tests.
    pub resolved_file: &'a dyn Fn(&str) -> Option<String>,
}

impl Planner<'_> {
    /// Plans `pane` from one observation. A recognized shell holding the pane gives a plain shell in its directory,
    /// `fallback_directory` when that cannot be read; every refusal of the restart replay, a daemon-leader shell
    /// included, refuses the copy (module docs for the lookup).
    ///
    /// # Errors
    ///
    /// The [`Refusal`] naming the pane and why it cannot be copied.
    pub fn resolve(&self, observation: &Observation, pane: &str, fallback_directory: &str) -> Result<Plan, Refusal> {
        let refusal = |reason| Refusal {
            pane: pane.to_owned(),
            reason,
        };
        if observation
            .foreground
            .as_ref()
            .and_then(PaneForeground::shell_name)
            .is_some()
        {
            let working_directory = observation
                .working_directory
                .clone()
                .unwrap_or_else(|| fallback_directory.to_owned());
            return Ok(Plan::Shell { working_directory });
        }
        let rules = self.lookup.rules;
        let argv = observation.foreground.as_ref().and_then(PaneForeground::command);
        let Some((argv, program)) = argv.and_then(|argv| Some((argv, argv.first()?))) else {
            return self.program(observation, pane);
        };
        if rules.is_absolute(program) {
            return self.program(observation, pane);
        }
        let Some(executable) = observation.executable_path.as_deref() else {
            return self.image_gone(observation, argv, pane);
        };
        if rules.has_separator(program) && observation.working_directory.is_none() {
            return Err(refusal(Reason::Replay(restart_replay::Refusal::NoDirectory)));
        }
        let image = (self.resolved_file)(executable).unwrap_or_else(|| executable.to_owned());
        let found: Vec<(String, String)> = self
            .starting_paths(program, observation)
            .into_iter()
            .filter_map(|path| (self.resolved_file)(&path).map(|resolved| (path, resolved)))
            .collect();
        let unreadable = observation.search_path.is_none() && !rules.has_separator(program);
        let fallback = if unreadable {
            Some(executable.to_owned())
        } else {
            found.first().map(|(path, _)| path.clone())
        };
        let started = found
            .iter()
            .find(|(_, resolved)| rules.same_file(resolved, &image))
            .map(|(path, _)| path.clone())
            .or(fallback)
            .ok_or_else(|| refusal(Reason::NotLocated))?;
        self.first_refusal([started, executable.to_owned()], observation, argv, pane)
    }

    /// A program whose image is gone starts what the pane's own `PATH` names now; a path-named one, or one with no
    /// `PATH` to read, is refused.
    fn image_gone(&self, observation: &Observation, argv: &[String], pane: &str) -> Result<Plan, Refusal> {
        let no_executable = || Refusal {
            pane: pane.to_owned(),
            reason: Reason::NoExecutable,
        };
        let program = &argv[0];
        if self.lookup.rules.has_separator(program) || observation.search_path.is_none() {
            return Err(no_executable());
        }
        let (started, target) = self
            .starting_paths(program, observation)
            .into_iter()
            .find_map(|path| (self.resolved_file)(&path).map(|target| (path, target)))
            .ok_or_else(no_executable)?;
        self.first_refusal([started, target], observation, argv, pane)
    }

    /// The plan of `argv` started as `started`, once it as `also` passes the same checks; the first refusal otherwise.
    fn first_refusal(
        &self,
        [started, also]: [String; 2],
        observation: &Observation,
        argv: &[String],
        pane: &str,
    ) -> Result<Plan, Refusal> {
        let plan = self.program(&running(observation, argv, &started), pane)?;
        self.program(&running(observation, argv, &also), pane)?;
        Ok(plan)
    }

    fn program(&self, observation: &Observation, pane: &str) -> Result<Plan, Refusal> {
        let refusal = |reason| Refusal {
            pane: pane.to_owned(),
            reason,
        };
        let launch = restart_replay::resolve(observation, self.shell, self.denylist)
            .map_err(|replay| refusal(Reason::Replay(replay)))?;
        if self.lookup.rules == PathRules::Posix && launch.argv.iter().any(|arg| arg.contains('\\')) {
            return Err(refusal(Reason::Backslash));
        }
        Ok(Plan::Program {
            argv: launch.argv,
            working_directory: launch.working_directory,
        })
    }

    /// Where `program` could have been started from: a path-named one under its directory; a bare name in each
    /// absolute entry of the process's `PATH` (a relative entry meant whatever directory the shell sat in at that
    /// lookup), the system directories without one.
    fn starting_paths(&self, program: &str, observation: &Observation) -> Vec<String> {
        let rules = self.lookup.rules;
        if rules.has_separator(program) {
            return observation
                .working_directory
                .as_deref()
                .map(|directory| self.lookup.spellings(rules.joined(program, directory)))
                .unwrap_or_default();
        }
        observation
            .search_path
            .as_deref()
            .unwrap_or(&self.lookup.system_search_path)
            .split(rules.list_separator())
            .map(|entry| entry.trim_matches('"'))
            .filter(|entry| rules.is_absolute(entry))
            .flat_map(|entry| self.lookup.spellings(rules.child(entry, program)))
            .collect()
    }
}

/// `observation` with its foreground `argv` started as `program`.
fn running(observation: &Observation, argv: &[String], program: &str) -> Observation {
    let argv = std::iter::once(program.to_owned())
        .chain(argv.iter().skip(1).cloned())
        .collect();
    Observation {
        foreground: Some(PaneForeground::Program(argv)),
        ..observation.clone()
    }
}

#[cfg(test)]
mod tests;
