//! A captured argv as one line typed at a shell's prompt, quoted for that shell: what a rerun restore types into a
//! pane's login shell (upstream's `LaunchSeed` types `CommandRestore.shellQuotedLine(argv) + "\n"`, a POSIX line,
//! whatever the shell; a Rust-only extension for the shells a Windows pane runs), and the PowerShell call a live
//! replay runs ([`crate::live::replay`]).
//!
//! The dialect is the pane's shell's, read off its program's name ([`ShellDialect::of_shell`]), a Windows `.exe`
//! dropped in any case:
//!
//! - **POSIX** (`sh`, `bash`, `zsh`, `dash`, `ksh` and kin, `tcsh`, `csh`): upstream's line, each word single-quoted
//!   ([`shell_quoted_line`]), a line feed after it. Git for Windows' `sh` and `bash` are POSIX shells too.
//! - **fish**: each word single-quoted the way fish reads it, where a backslash escapes another backslash or a quote
//!   ([`fish_quoted`]); a POSIX word with a backslash before its closing quote would not end there.
//! - **PowerShell** (7's `pwsh`, Windows PowerShell 5.1's `powershell`): the call operator and each word single-quoted
//!   (`''` doubling, [`powershell_quoted`]), a carriage return after it, which `PSReadLine` takes for Enter (a line
//!   feed is Ctrl+J there): `& 'program' 'argument'`. The call operator always, as a bare first word can be a keyword
//!   or a number. A native program receives its arguments through PowerShell's argument passing, which differs by
//!   version and preference (`$PSNativeCommandArgumentPassing`): 5.1 and 7.0–7.2 have only the legacy one, which drops
//!   an empty argument and loses an embedded double quote. So an argv any of the passings would change runs in a
//!   scope of its own under `Legacy`, each argument already quoted for the C runtime's split ([`crt_quoted`]): legacy
//!   passes such a word as it is (its whitespace sits inside its quotes), so the program's split rebuilds the argv on
//!   every version and OS (.NET's own split on Linux and macOS follows the same rules). An argv of plain words passes
//!   the same under every passing and is typed without that scope. Exact for programs that split their command line
//!   as the C runtime does (`CommandLineToArgvW`); `cmd.exe`, batch files and `msiexec` parse theirs their own way. A
//!   bare program name runs what PowerShell finds for it first, an alias or a function before a program, as it did
//!   when typed.
//! - **cmd** (`cmd.exe`): the argv quoted for the C runtime's split, then escaped for `cmd.exe`'s own reading of a
//!   prompt line, a carriage return after it. `cmd.exe` toggles its quoted state at every `"`, and only outside quotes
//!   does a caret escape, so every quote of the arguments is typed as `^"` and `cmd.exe` never enters its quoted state
//!   there: each of `^ & | < > ( ) !` is caret-escaped, and each `%` is followed by a caret, which keeps `%NAME%` from
//!   naming a variable (the prompt has no `%%`). The program is typed bare when it is a plain path, else in real
//!   quotes; one holding a `%` or a `"` is refused, as no quoting keeps it whole. Limits: a program reading its
//!   command line as the C runtime does gets the exact argv; a batch file or an internal command of the same name as
//!   the program (`echo`, `start`) does not; with delayed expansion turned on (`cmd /V:ON`, or the registry's
//!   `DelayedExpansion`) a `!` loses its caret and a `!NAME!` expands; a line over 8191 characters is refused by
//!   `cmd.exe` itself.
//! - **Unknown** (`nu`, `xonsh`, `elvish`, a config `command` naming another program): the safest choice is typing
//!   only what every dialect reads alike: plain words (ASCII letters and digits, `_ - . / : +`, on Windows `\` too,
//!   none empty), bare, with a carriage return, Enter's own key code. An argv with any other word is not typed: the
//!   pane comes up as a plain shell, as for a denylisted program, rather than one whose line a shell could run
//!   differently.

use crate::command_restore::{basename, shell_name, shell_quoted_line};

/// The POSIX-family shells, by name: upstream's and their kin.
const POSIX_SHELLS: [&str; 13] = [
    "sh", "bash", "zsh", "dash", "ash", "ksh", "mksh", "oksh", "pdksh", "yash", "busybox", "tcsh", "csh",
];

/// What `cmd.exe` reads specially outside quotes, a caret escaping each; `%` and `"` have rules of their own.
const CMD_METACHARACTERS: [char; 7] = ['^', '&', '|', '<', '>', '(', ')'];

/// How a shell reads a typed line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellDialect {
    Posix,
    Fish,
    PowerShell,
    Cmd,
    Unknown,
}

impl ShellDialect {
    /// The dialect of the shell program `name` (a path or a bare name, a login `-` and a Windows `.exe` dropped).
    #[must_use]
    pub fn of_program(name: &str) -> Self {
        let name = basename(name.strip_prefix('-').unwrap_or(name));
        let name = shell_name(name).to_ascii_lowercase();
        match name.as_str() {
            "fish" => Self::Fish,
            "pwsh" | "powershell" => Self::PowerShell,
            "cmd" => Self::Cmd,
            posix if POSIX_SHELLS.contains(&posix) => Self::Posix,
            _ => Self::Unknown,
        }
    }

    /// The dialect of a pane's shell as its config names it: a program path that may hold spaces
    /// (`C:\Program Files\PowerShell\7\pwsh.exe`) or a program and its arguments (`pwsh -NoLogo`, `/usr/bin/fish -l`).
    /// The first of the whole value's last component, that component's first word and the value's first word that
    /// names a known shell; [`Self::Unknown`] when none does.
    #[must_use]
    pub fn of_shell(shell: &str) -> Self {
        let first_word = |value: &str| value.split_whitespace().next().unwrap_or(value).to_owned();
        let last = basename(shell).to_owned();
        [last.clone(), first_word(&last), first_word(shell)]
            .iter()
            .map(|candidate| Self::of_program(candidate))
            .find(|dialect| *dialect != Self::Unknown)
            .unwrap_or(Self::Unknown)
    }

    /// What submits a typed line: upstream's line feed for POSIX shells and fish, else a carriage return, the Enter
    /// key's own code.
    #[must_use]
    pub fn enter(self) -> &'static str {
        match self {
            Self::Posix | Self::Fish => "\n",
            Self::PowerShell | Self::Cmd | Self::Unknown => "\r",
        }
    }

    /// `argv` as one line this dialect runs with exactly those words, without its Enter; `None` when it cannot be
    /// typed safely (module docs) or is empty.
    #[must_use]
    pub fn line<S: AsRef<str>>(self, argv: &[S]) -> Option<String> {
        let (program, arguments) = argv.split_first()?;
        let program = program.as_ref();
        match self {
            Self::Posix => Some(shell_quoted_line(argv)),
            Self::Fish => Some(words(argv, fish_quoted)),
            Self::PowerShell => Some(powershell_call(argv)),
            Self::Cmd => cmd_line(program, arguments),
            Self::Unknown => argv
                .iter()
                .all(|word| is_portable(word.as_ref()))
                .then(|| words(argv, str::to_owned)),
        }
    }

    /// [`Self::line`] and its [`Self::enter`].
    #[must_use]
    pub fn typed<S: AsRef<str>>(self, argv: &[S]) -> Option<String> {
        self.line(argv).map(|line| line + self.enter())
    }
}

fn words<S: AsRef<str>>(argv: &[S], quote: impl Fn(&str) -> String) -> String {
    argv.iter()
        .map(|word| quote(word.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `word` reads alike in every dialect, bare.
fn is_portable(word: &str) -> bool {
    !word.is_empty()
        && word.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '+') || (cfg!(windows) && c == '\\')
        })
}

/// `value` as one PowerShell word: a single-quoted string, inside which a single quote is written twice. PowerShell
/// takes the typographic single quotes (U+2018–U+201B) for one as well, so those are doubled too.
#[must_use]
pub fn powershell_quoted(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for character in value.chars() {
        if matches!(character, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}') {
            quoted.push(character);
        }
        quoted.push(character);
    }
    quoted.push('\'');
    quoted
}

/// `value` as one fish word: single quotes, inside which only a backslash and a single quote are escaped.
#[must_use]
pub fn fish_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\\', r"\\").replace('\'', r"\'"))
}

/// The PowerShell statement that runs `argv` as a native program with exactly those arguments (module docs): `& 'p'
/// 'a'`, or, when an argument needs quoting for the C runtime's split, `& { $PSNativeCommandArgumentPassing =
/// 'Legacy' ; & 'p' '"a b"' }`. Its own scope keeps the user's preference as it was.
#[must_use]
pub fn powershell_call<S: AsRef<str>>(argv: &[S]) -> String {
    let Some((program, arguments)) = argv.split_first() else {
        return String::new();
    };
    let encoded: Vec<String> = arguments.iter().map(|word| crt_quoted(word.as_ref())).collect();
    let mut call = format!("& {}", powershell_quoted(program.as_ref()));
    for word in &encoded {
        call.push(' ');
        call.push_str(&powershell_quoted(word));
    }
    let plain = encoded
        .iter()
        .zip(arguments)
        .all(|(word, argument)| word == argument.as_ref());
    if plain {
        call
    } else {
        format!("& {{ $PSNativeCommandArgumentPassing = 'Legacy' ; {call} }}")
    }
}

/// `value` as one word of a command line the C runtime splits (`CommandLineToArgvW`): as it is when it is not empty and
/// holds no space, tab or double quote; else in double quotes, each inner quote and the backslashes before it
/// escaped, the trailing backslashes doubled.
#[must_use]
pub fn crt_quoted(value: &str) -> String {
    if !value.is_empty() && !value.contains([' ', '\t', '"']) {
        return value.to_owned();
    }
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    let mut backslashes = 0;
    for character in value.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            other => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(other);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

/// The `cmd.exe` prompt line of `program` and `arguments` (module docs); `None` when the program holds a `%` or a `"`.
fn cmd_line<S: AsRef<str>>(program: &str, arguments: &[S]) -> Option<String> {
    if program.is_empty() || program.contains(['%', '"']) {
        return None;
    }
    let bare = program.starts_with(|c: char| c.is_ascii_alphanumeric())
        && program
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '\\' | ':'));
    let mut line = if bare {
        program.to_owned()
    } else {
        format!("\"{program}\"")
    };
    let mut after_percent = false;
    for word in arguments {
        let encoded = format!(" {}", crt_quoted(word.as_ref()));
        for character in encoded.chars() {
            let escaped = character == '"' || character == '!' || CMD_METACHARACTERS.contains(&character);
            if escaped || after_percent {
                line.push('^');
            }
            line.push(character);
            after_percent = character == '%';
        }
    }
    Some(line)
}

#[cfg(test)]
mod tests;
