//! A captured argv as one line typed at a shell's prompt, quoted for that shell: what a rerun restore types into a
//! pane's login shell (upstream's `LaunchSeed` types `CommandRestore.shellQuotedLine(argv) + "\n"`, a POSIX line,
//! whatever the shell; a Rust-only extension for the shells a Windows pane runs), and the PowerShell call a live
//! replay runs ([`crate::live::replay`]).
//!
//! The dialect is the pane's shell's, read off its program's name ([`ShellDialect::of_program`], of a `/bin/sh` line's
//! program as [`shell_program`] reads it, past an `env` that runs it), a Windows `.exe` dropped in any case:
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

use std::borrow::Cow;
use std::collections::VecDeque;

use crate::command_restore::{basename, shell_name, shell_quoted_line};

/// The POSIX-family shells, by name: upstream's and their kin (the Korn shells' `ksh93`, OpenBSD's ports `oksh` and
/// `loksh`, `mksh`'s legacy `lksh`, restricted `rbash`, Debian's `posh`, the Schily `bosh`, `BusyBox`'s `ash` and
/// `hush`).
const POSIX_SHELLS: [&str; 20] = [
    "sh", "bash", "rbash", "zsh", "dash", "ash", "hush", "ksh", "ksh93", "mksh", "lksh", "oksh", "loksh", "pdksh",
    "posh", "bosh", "yash", "busybox", "tcsh", "csh",
];

/// Whether `word` names `env` (by any path, a Windows `.exe` dropped in any case).
fn is_env(word: &str) -> bool {
    shell_name(basename(word)).eq_ignore_ascii_case("env")
}

/// The program that `env` runs after its options, assignments, split strings and nested wrappers, the words read as
/// `/bin/sh` reads them (quotes removed) and `-S` text as `env` splits it; `None` when it runs none, when a shell
/// expansion or an `env` escape could change it, or when the rest of the line could start another command.
fn env_program(arguments: &str) -> Option<String> {
    let mut line = EnvLine {
        rest: arguments,
        pending: VecDeque::new(),
    };
    loop {
        let mut word = line.next()?;
        while word.starts_with('-') && word != "-" {
            if word == "--" {
                word = line.next()?;
                break;
            }
            line.option(&word)?;
            word = line.next()?;
        }
        if word == "-" {
            word = line.next()?;
        }
        while word.contains('=') {
            word = line.next()?;
        }
        if !is_env(&word) {
            return ends_plainly(line.rest).then_some(word);
        }
    }
}

/// Whether `/bin/sh` runs `rest`, the text after a program, as that program's arguments, with no other command after
/// it.
fn ends_plainly(mut rest: &str) -> bool {
    loop {
        match next_word(rest, Lexer::After) {
            Ok(Some((_, after))) => rest = after,
            Ok(None) => return true,
            Err(Refused) => return false,
        }
    }
}

/// An `env` line read one word at a time: the words an `env -S` split gave first, then the shell's.
struct EnvLine<'a> {
    rest: &'a str,
    pending: VecDeque<String>,
}

impl EnvLine<'_> {
    fn next(&mut self) -> Option<String> {
        if let Some(word) = self.pending.pop_front() {
            return Some(word);
        }
        let (word, rest) = next_word(self.rest, Lexer::Before).ok()??;
        self.rest = rest;
        Some(word)
    }

    /// Reads one option word of `env` (`-iuNAME`, `--chdir=/tmp`, `--arg bash`); `None` when `env` would not run a
    /// program after it.
    fn option(&mut self, word: &str) -> Option<()> {
        match word.strip_prefix("--") {
            Some(long) => self.long_option(long),
            None => self.short_options(&word[1..]),
        }
    }

    /// A long option, by any unique prefix of its name as `getopt_long` reads it.
    fn long_option(&mut self, long: &str) -> Option<()> {
        let (name, value) = match long.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (long, None),
        };
        let takes = long_option_takes(name)?;
        match (takes, value) {
            (Takes::Exit, _) | (Takes::Nothing, Some(_)) => None,
            (Takes::Nothing | Takes::Optional, _) => Some(()),
            (Takes::Value | Takes::Split, Some(value)) => self.valued(takes, value),
            (Takes::Value | Takes::Split, None) => {
                let value = self.next()?;
                self.valued(takes, &value)
            }
        }
    }

    fn valued(&mut self, takes: Takes, value: &str) -> Option<()> {
        if takes == Takes::Split {
            self.split(value)?;
        }
        Some(())
    }

    /// A bundle of short options without its `-`: GNU's and the BSDs' (`-P`).
    fn short_options(&mut self, bundle: &str) -> Option<()> {
        for (at, option) in bundle.char_indices() {
            let rest = &bundle[at + option.len_utf8()..];
            let takes = match option {
                'i' | 'v' | '0' => continue,
                'u' | 'C' | 'P' | 'a' => Takes::Value,
                'S' => Takes::Split,
                _ => return None,
            };
            let value = if rest.is_empty() { self.next()? } else { rest.to_owned() };
            return self.valued(takes, &value);
        }
        Some(())
    }

    /// Puts the words of `env -S`'s `text` before the rest of the line.
    fn split(&mut self, text: &str) -> Option<()> {
        // env reads an escape or a variable in its own way.
        if text.contains(['\\', '$']) {
            return None;
        }
        let mut words = Vec::new();
        let mut rest = text;
        while let Some((word, after)) = next_word(rest, Lexer::Split).ok()? {
            words.push(word);
            rest = after;
        }
        for word in words.into_iter().rev() {
            self.pending.push_front(word);
        }
        Some(())
    }
}

/// What a long option of GNU `env` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Takes {
    Nothing,
    /// A value only after `=`.
    Optional,
    Value,
    /// Text that `env` splits into more words.
    Split,
    /// None: `env` exits without a program.
    Exit,
}

const LONG_OPTIONS: [(&str, Takes); 13] = [
    ("argv0", Takes::Value),
    ("block-signal", Takes::Optional),
    ("chdir", Takes::Value),
    ("debug", Takes::Nothing),
    ("default-signal", Takes::Optional),
    ("help", Takes::Exit),
    ("ignore-environment", Takes::Nothing),
    ("ignore-signal", Takes::Optional),
    ("list-signal-handling", Takes::Nothing),
    ("null", Takes::Nothing),
    ("split-string", Takes::Split),
    ("unset", Takes::Value),
    ("version", Takes::Exit),
];

/// The long option that `name` names in full or by a unique prefix.
fn long_option_takes(name: &str) -> Option<Takes> {
    if let Some(&(_, takes)) = LONG_OPTIONS.iter().find(|(option, _)| *option == name) {
        return Some(takes);
    }
    let mut matches = LONG_OPTIONS.iter().filter(|(option, _)| option.starts_with(name));
    let &(_, takes) = matches.next()?;
    matches.next().is_none().then_some(takes)
}

/// How a word of an `env` line is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lexer {
    /// By `/bin/sh`, before the program: each expansion could change the program, so none is accepted.
    Before,
    /// By `/bin/sh`, after the program: an expansion changes only the program's arguments, but what could start
    /// another command is refused.
    After,
    /// By `env -S`, which expands no globs, tildes or braces (the caller refuses its escapes and variables).
    Split,
}

/// A word that the lexer cannot read with certainty.
#[derive(Debug)]
struct Refused;

impl Lexer {
    fn ends_word(self, character: char) -> bool {
        match self {
            Self::Before | Self::After => matches!(character, ' ' | '\t' | '\n'),
            Self::Split => matches!(character, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r'),
        }
    }

    fn plain(self, character: char, previous: Option<char>) -> Result<(), Refused> {
        let refused = match self {
            Self::Before => "{}*?[]~()<>|&;`".contains(character),
            Self::After => {
                "();`".contains(character) || (matches!(character, '|' | '&') && !matches!(previous, Some('<' | '>')))
            }
            Self::Split => false,
        };
        if refused { Err(Refused) } else { Ok(()) }
    }

    /// A `$` before `next`: after the program, a plain parameter expands to the program's arguments only.
    fn dollar(self, next: Option<char>) -> Result<(), Refused> {
        if self == Self::After && !matches!(next, Some('(' | '{' | '\'' | '"')) {
            Ok(())
        } else {
            Err(Refused)
        }
    }
}

type Characters<'a> = std::iter::Peekable<std::str::CharIndices<'a>>;

/// The next word of `text` with its quotes removed, and the text after it; `None` at the end of the line, at a shell
/// comment that only blank text follows, or at an `env -S` comment; [`Refused`] when the lexer cannot read the word
/// with certainty (an expansion that `lexer` refuses, an unterminated quote, a line continuation) or another command
/// could follow.
fn next_word(text: &str, lexer: Lexer) -> Result<Option<(String, &str)>, Refused> {
    let text =
        text.trim_start_matches(|character| lexer.ends_word(character) && (lexer == Lexer::Split || character != '\n'));
    let text = match text.strip_prefix('#') {
        // The shell's comment ends at the line's end; env -S's ends the split text.
        Some(comment) if lexer != Lexer::Split => comment.find('\n').map_or("", |at| &comment[at..]),
        Some(_) => return Ok(None),
        None => text,
    };
    if text.starts_with('\n') {
        // A newline ends the shell's command: only blank text may follow it.
        return if text.trim_start_matches([' ', '\t', '\n']).is_empty() {
            Ok(None)
        } else {
            Err(Refused)
        };
    }
    if text.is_empty() {
        return Ok(None);
    }
    let mut word = String::new();
    let mut characters = text.char_indices().peekable();
    let mut previous = None;
    while let Some((at, character)) = characters.next() {
        if lexer.ends_word(character) {
            return Ok(Some((word, &text[at..])));
        }
        match character {
            '\'' => single_quoted(&mut characters, &mut word)?,
            '"' => double_quoted(&mut characters, &mut word, lexer)?,
            '\\' => match characters.next() {
                Some((_, next)) if next != '\n' => word.push(next),
                _ => return Err(Refused),
            },
            '$' => {
                lexer.dollar(characters.peek().map(|&(_, next)| next))?;
                word.push('$');
            }
            _ => {
                lexer.plain(character, previous)?;
                word.push(character);
            }
        }
        previous = Some(character);
    }
    Ok(Some((word, "")))
}

fn single_quoted(characters: &mut Characters<'_>, word: &mut String) -> Result<(), Refused> {
    for (_, character) in characters.by_ref() {
        if character == '\'' {
            return Ok(());
        }
        word.push(character);
    }
    Err(Refused)
}

fn double_quoted(characters: &mut Characters<'_>, word: &mut String, lexer: Lexer) -> Result<(), Refused> {
    while let Some((_, character)) = characters.next() {
        match character {
            '"' => return Ok(()),
            '`' if lexer != Lexer::Split => return Err(Refused),
            '$' => {
                lexer.dollar(characters.peek().map(|&(_, next)| next))?;
                word.push('$');
            }
            '\\' => match characters.next().ok_or(Refused)? {
                (_, '\n') => return Err(Refused),
                (_, next @ ('$' | '`' | '"' | '\\')) => word.push(next),
                (_, next) => {
                    word.push('\\');
                    word.push(next);
                }
            },
            _ => word.push(character),
        }
    }
    Err(Refused)
}

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

    /// Whether the shell program `name` reads a bracketed paste at its prompt, once its line editor turns mode 2004
    /// on: bash 5.1 and later, zsh 5.1 and later, and fish. Never on Windows, where the pseudoconsole decides what
    /// reaches the shell.
    #[must_use]
    pub fn reads_bracketed_paste(name: &str) -> bool {
        let name = basename(name.strip_prefix('-').unwrap_or(name));
        !cfg!(windows) && matches!(name, "bash" | "rbash" | "zsh" | "fish")
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

/// The program of a pane's shell as its config names it: a program and its arguments (`pwsh -NoLogo`,
/// `/usr/bin/fish -l`); its dialect is [`ShellDialect::of_program`]'s. Outside Windows, `/bin/sh` runs the value, so
/// the program is its first word as `/bin/sh` reads it (`'/opt/my tools/fish' -l`), past the variable assignments
/// before it (`X=1 fish`); `None` when an expansion could change that word, or when `/bin/sh` could run another
/// command after it (`;`, `|`, `&`, a newline, `$(…)`: `bash -c : ; tcsh`). A shell's own `-c` command is taken as
/// that shell's. A shell that runs directly, not through `/bin/sh` (a profile's executable, Live's `SHELL`), is its
/// own program: do not read it here. Windows runs the value directly, so there the program is the value's shortest
/// run of leading words that names a known shell or `env`: a path with spaces is read whole
/// (`C:\Program Files\PowerShell\7\pwsh.exe`) and a shell's argument never stands for it
/// (`pwsh -File C:\scripts\bash`). A program `env` is the program it runs (`env_program`: `/usr/bin/env zsh`,
/// `env -S 'zsh -l'`, `C:\Program Files\Git\usr\bin\env.exe bash`).
#[must_use]
pub fn shell_program(shell: &str) -> Option<Cow<'_, str>> {
    let mut rest = shell.trim_start_matches(BLANKS);
    if cfg!(windows) {
        return windows_shell_program(rest);
    }
    let (program, rest) = loop {
        let text = rest.trim_start_matches([' ', '\t']);
        // A `~/` prefix expands to the home directory and keeps the program's name.
        let text = text
            .strip_prefix('~')
            .filter(|path| path.starts_with('/'))
            .unwrap_or(text);
        let (word, after) = next_word(text, Lexer::Before).ok()??;
        if !is_assignment(text) {
            break (word, after);
        }
        rest = after;
    };
    if is_env(&program) {
        return env_program(rest).map(Cow::Owned);
    }
    (ShellDialect::of_program(&program) != ShellDialect::Unknown && ends_plainly(rest)).then_some(Cow::Owned(program))
}

/// Whether `text` starts with a word that `/bin/sh` reads as a variable assignment (`NAME=value`) before the program.
fn is_assignment(text: &str) -> bool {
    let name = text
        .find(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .unwrap_or(text.len());
    name > 0 && !text.starts_with(|character: char| character.is_ascii_digit()) && text[name..].starts_with('=')
}

/// `/bin/sh` splits words only at these, not at other Unicode white space.
const BLANKS: [char; 3] = [' ', '\t', '\n'];

/// The program of a shell value that Windows runs directly: its shortest run of leading words that names a known shell
/// or `env`.
fn windows_shell_program(shell: &str) -> Option<Cow<'_, str>> {
    let mut end = 0;
    loop {
        let rest = &shell[end..];
        let word = rest.trim_start_matches(BLANKS);
        if word.is_empty() {
            return None;
        }
        end += rest.len() - word.len() + word.find(BLANKS).unwrap_or(word.len());
        let program = &shell[..end];
        if is_env(program) {
            return env_program(&shell[end..]).map(Cow::Owned);
        }
        if ShellDialect::of_program(program) != ShellDialect::Unknown {
            return Some(Cow::Borrowed(program));
        }
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
