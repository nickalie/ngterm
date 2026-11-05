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
    #[arg(long, help = "The file's name, as the user picked it.")]
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
