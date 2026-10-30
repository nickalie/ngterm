//! `ngtermctl version` (`agtermctlKit/MiscCommands.swift` `Version`): the serving app's `version (commit)`, then this
//! CLI's own executable.

use std::path::PathBuf;
use std::{env, fs};

use ngterm_core::paths::Env;
use ngterm_proto::{Command, Request};

use crate::{BasicOptions, Failure, RequestCommand};

/// `ngtermctl version [--socket <socket>] [--json]`.
#[derive(Debug, clap::Args)]
#[command(about = "Print the version of the app serving the socket.")]
pub struct Version {
    #[command(flatten)]
    options: BasicOptions,
}

impl RequestCommand for Version {
    fn options(&self) -> &BasicOptions {
        &self.options
    }

    fn make_request(&self) -> Request {
        Request::new(Command::Version)
    }
}

impl Version {
    /// The app's version, then `client: <path>` unless `--json`, so a caller can tell which CLI answered.
    pub fn run(&self, env: &dyn Env) -> Result<(), Failure> {
        self.default_run(env)?;
        if !self.options.json
            && let Some(path) = client_path()
        {
            println!("client: {}", path.display());
        }
        Ok(())
    }
}

/// The running executable's real path: the kernel's answer (`/proc/self/exe`), not `argv[0]`, resolved through
/// symlinks since an installed CLI is usually one. On Windows without the verbatim prefix `canonicalize` adds
/// (`\\?\C:\...` reads `C:\...`, `\\?\UNC\server\...` reads `\\server\...`), as the user would spell it.
pub fn client_path() -> Option<PathBuf> {
    env::current_exe()
        .and_then(fs::canonicalize)
        .ok()
        .map(without_verbatim_prefix)
}

#[cfg(unix)]
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    path
}

#[cfg(windows)]
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{share}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::path::PathBuf;

    use super::without_verbatim_prefix;

    #[test]
    fn a_verbatim_path_reads_as_the_user_spells_it() {
        let cases = [
            (r"\\?\C:\Users\u\ngtermctl.exe", r"C:\Users\u\ngtermctl.exe"),
            (r"\\?\UNC\server\share\ngtermctl.exe", r"\\server\share\ngtermctl.exe"),
            (r"\\?\Volume{0}\ngtermctl.exe", r"\\?\Volume{0}\ngtermctl.exe"),
            (r"C:\plain.exe", r"C:\plain.exe"),
        ];
        for (canonical, shown) in cases {
            assert_eq!(without_verbatim_prefix(PathBuf::from(canonical)), PathBuf::from(shown));
        }
    }
}
