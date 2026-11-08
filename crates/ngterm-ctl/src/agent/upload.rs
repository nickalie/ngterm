//! `ngtermctl agent upload`: a file the Android chat composer attaches, taken on standard input and stored in this
//! machine's state directory for the pane's agent to read ([`ngterm_core::agent_chat::upload`]); it prints the stored
//! path, which the phone puts into the prompt.

use std::io::{self, Write};
use std::time::SystemTime;

use ngterm_core::agent_chat::upload::{UPLOAD_CAP, prune, store, uploads_dir};
use ngterm_core::paths::{Env, state_dir};

use crate::{Failure, client};

/// `ngtermctl agent upload --name NAME --size BYTES`: standard input's bytes, stored; prints the stored path.
#[derive(Debug, clap::Args)]
#[command(
    about = "Store a file from standard input for an agent to read, and print its path.",
    long_about = "Reads exactly --size bytes from standard input and stores them as --name (its last path component, \
                  with what an OS can't take in a file name replaced) in a fresh directory under the state \
                  directory's uploads/, then prints the stored file's full path. Input that ends early or runs past \
                  --size stores nothing. Uploads older than a week are removed first. At most 32 MiB."
)]
pub struct Upload {
    #[arg(
        long,
        allow_hyphen_values = true,
        help = "The file's name, as the user picked it; one that starts with `-` is still the name."
    )]
    name: String,

    #[arg(long, value_name = "BYTES", help = "How many bytes standard input holds.")]
    size: u64,
}

impl Upload {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.size > UPLOAD_CAP {
            return Err(format!("--size is over the {UPLOAD_CAP}-byte cap"));
        }
        Ok(())
    }

    pub(super) fn run(&self, env: &dyn Env) -> Result<(), Failure> {
        let dir = uploads_dir(&state_dir(env));
        let now = SystemTime::now();
        prune(&dir, now);
        let path = store(&dir, &self.name, self.size, &mut io::stdin().lock(), now)
            .map_err(|error| Failure::Local(error.to_string()))?;
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{}", path.display())
            .and_then(|()| stdout.flush())
            .map_err(|error| Failure::Local(format!("could not write the path: {}", client::os_message(&error))))
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::*;
    use crate::agent::{Agent, AgentCommand};
    use crate::{Command as Cli, Ngtermctl};

    fn upload(argv: &[&str]) -> Result<Upload, clap::Error> {
        let argv = ["ngtermctl", "agent", "upload"].iter().chain(argv).copied();
        match Ngtermctl::try_parse_from(argv)?.command {
            Cli::Agent(Agent {
                command: AgentCommand::Upload(upload),
            }) => Ok(upload),
            _ => panic!("not agent upload"),
        }
    }

    #[test]
    fn a_name_that_starts_with_a_dash_is_the_name() {
        for argv in [
            &["--name", "-notes.txt", "--size", "3"][..],
            &["--name=-notes.txt", "--size", "3"],
        ] {
            let parsed = upload(argv).unwrap();
            assert_eq!((parsed.name.as_str(), parsed.size), ("-notes.txt", 3), "{argv:?}");
        }
    }
    #[test]
    fn upload_sizes_include_zero_and_the_cap_but_refuse_larger_inputs() {
        for size in [0, UPLOAD_CAP] {
            assert_eq!(
                Upload {
                    name: "x".to_owned(),
                    size
                }
                .validate(),
                Ok(())
            );
        }
        assert_eq!(
            Upload {
                name: "x".to_owned(),
                size: UPLOAD_CAP + 1
            }
            .validate(),
            Err(format!("--size is over the {UPLOAD_CAP}-byte cap"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_path_write_failure_keeps_the_stored_file_and_reports_the_error() {
        super::super::commands::tests::with_full_stdout(
            "agent::upload::tests::a_path_write_failure_keeps_the_stored_file_and_reports_the_error",
            || {
                let directory = tempfile::tempdir().unwrap();
                let env = ngterm_core::paths::MapEnv::default().with("NGTERM_STATE_DIR", directory.path());
                let upload = Upload {
                    name: "empty.txt".to_owned(),
                    size: 0,
                };
                assert!(matches!(upload.run(&env), Err(Failure::Local(message))
                    if message == "could not write the path: No space left on device"));
                let uploads = uploads_dir(directory.path());
                let stored: Vec<_> = std::fs::read_dir(uploads).unwrap().map(Result::unwrap).collect();
                assert_eq!(stored.len(), 1);
                assert_eq!(std::fs::read(stored[0].path().join("empty.txt")).unwrap(), b"");
            },
        );
    }
}
