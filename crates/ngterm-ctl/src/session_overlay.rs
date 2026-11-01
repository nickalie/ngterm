//! `ngtermctl session overlay ...` (`agtermctlKit/SessionCommands.swift` `Session.Overlay`): the ephemeral overlay
//! over a session or one of its split panes. This module holds the group and its program side (`open`, `close`,
//! `resize`, `result`, `copy`, `text`); [`crate::overlay_page`] holds the HTML page side (`reload`, `submit`,
//! `navigate`, the page `--block` and `result --page`), and [`crate::overlay_job`] the `run-job` helper.
//!
//! `open --block` on a program opens it, then polls `session.overlay.result` by the id the open returned until the
//! program ends, and exits with its status, printing nothing of its own (the server's line under `--json`).

use std::process::ExitCode;
use std::time::Duration;

use ngterm_core::html;
use ngterm_core::model::is_valid_color_hex;
use ngterm_core::paths::Env;
use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Args, Command, Request};

use crate::client::Reply;
use crate::modal::{LiveIo, ModalIo};
use crate::overlay_page::{self, PageRunner, control_result};
use crate::session::{flag, session_request, validate_size_percent};
use crate::session_hud::validate_overlay_pane;
use crate::{BasicOptions, ClientOptions, Failure, RequestCommand, TargetOptions, output};

/// The refusal a program result reads while the program runs (upstream's `OverlayResultError.stillRunning`).
pub const STILL_RUNNING: &str = "overlay still running";
/// The pause between two `--block` polls of a running program.
const PROGRAM_POLL_DELAY: Duration = Duration::from_millis(100);

/// Upstream's `Open.blockHelp`.
const BLOCK_HELP: &str = "Block until COMMAND exits and exit with its status (the program renders normally; capture its \
                          output via the program's own output file). With --html, wait for the page to submit or \
                          close, print its outcome JSON, and exit 0 when submitted, 2 when dismissed, 1 on error.";
/// Upstream's `Open.persistentHelp`.
const PERSISTENT_HELP: &str = "With --url, keep cookies and site data in the saved browser store shared by all \
                               persistent pages, so a login can outlive the overlay and an app restart; without it \
                               each page has its own in-memory store. `browser clear` empties it.";
/// Upstream's `Open.browseHelp`.
const BROWSE_HELP: &str = "With --url, let the page leave its first site: links, redirects and scripts may take it to \
                           any http or https address, and the strip names the site shown. Without it the page stays \
                           on the site it opened.";

/// `ngtermctl session overlay <subcommand>`.
#[derive(Debug, clap::Args)]
#[command(
    about = "Open, read, resize, or close an ephemeral overlay terminal on a session.",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Overlay {
    #[command(subcommand)]
    command: OverlayCommand,
}

impl Overlay {
    /// The subcommand this invocation runs.
    pub fn command(&self) -> &dyn RequestCommand {
        match &self.command {
            OverlayCommand::Open(command) => command.as_ref(),
            OverlayCommand::Close(command) => command,
            OverlayCommand::Hide(command) => command,
            OverlayCommand::Show(command) => command,
            OverlayCommand::Toggle(command) => command,
            OverlayCommand::Resize(command) => command,
            OverlayCommand::Reload(command) => command,
            OverlayCommand::Navigate(command) => command,
            OverlayCommand::Result(command) => command,
            OverlayCommand::Submit(command) => command,
            OverlayCommand::Copy(command) => command,
            OverlayCommand::Text(command) => command,
            OverlayCommand::RunJob(command) => command,
            OverlayCommand::PageServe(command) => command,
            OverlayCommand::PageAttach(command) => command,
        }
    }
}

/// The subcommands, in upstream's `Overlay.configuration.subcommands` order. `open` is boxed: its many options would
/// otherwise make every parsed command as large.
#[derive(Debug, clap::Subcommand)]
enum OverlayCommand {
    Open(Box<Open>),
    Close(Close),
    Hide(Hide),
    Show(Show),
    Toggle(Toggle),
    Resize(Resize),
    Reload(overlay_page::Reload),
    Navigate(overlay_page::Navigate),
    Result(ResultCommand),
    Submit(overlay_page::Submit),
    Copy(Copy),
    Text(Text),
    RunJob(crate::overlay_job::RunJob),
    PageServe(PageServe),
    PageAttach(PageAttach),
}

/// The exit status of a program status: its low byte, as the OS keeps of an `exit(code)`.
fn exit_byte(code: i64) -> u8 {
    u8::try_from(code.rem_euclid(256)).unwrap_or(u8::MAX)
}

// MARK: - open

/// `ngtermctl session overlay open [<command>]`: a program, or a page with `--html`/`--url`; `--background-color` rides
/// the wire as `color`.
#[derive(Debug, clap::Args)]
#[command(
    about = "Open an overlay running COMMAND (it closes when COMMAND exits), or showing a page with --html or --url."
)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the flags are upstream's, one field each, in the order its help lists them"
)]
pub struct Open {
    #[arg(help = "Program to run in the overlay (e.g. revdiff); omit with --html or --url.")]
    command: Option<String>,

    #[arg(long, help = "Show this local HTML file instead of running COMMAND.")]
    html: Option<String>,

    #[arg(
        long,
        help = "Show this http or https URL instead of running COMMAND; links to its own origin load in place. \
                localhost means the Mac running ngterm."
    )]
    url: Option<String>,

    #[arg(
        long,
        help = "With --html or --url, add back, forward, reload, open in browser, and Show in Finder or Copy Link \
                buttons."
    )]
    navigation: bool,

    #[arg(
        long = "js",
        help = "With --html or --url, let the page run its own JavaScript (off by default)."
    )]
    javascript: bool,

    #[arg(
        long,
        help = "With --html, or --url on 127.0.0.1, ::1 or localhost, show the page without ngterm's strip naming \
                it; ⌘W or session overlay close closes it."
    )]
    chromeless: bool,

    #[arg(long, help = PERSISTENT_HELP)]
    persistent: bool,

    #[arg(long, help = BROWSE_HELP)]
    browse: bool,

    #[arg(
        long,
        help = "Working directory (default: the session's current directory). With --html, grants read access \
                inside this directory; relative links resolve beside FILE. Without --cwd, the page has no file access."
    )]
    cwd: Option<String>,

    #[arg(long, help = "Keep the overlay open after COMMAND exits (press any key to close).")]
    wait: bool,

    #[arg(long, help = BLOCK_HELP)]
    block: bool,

    #[arg(
        long,
        help = "Open the --html/--url page as a page companion under the target session: a sidebar row of its own, \
                full size, that closes with its page. Refuses --pane, --size-percent, --width and --height."
    )]
    companion: bool,

    #[arg(
        long,
        help = "Select (switch to) the target session after opening the overlay (default: open without switching)."
    )]
    follow: bool,

    #[arg(
        long,
        help = "Render a floating, framed panel at PERCENT (1-100) of the pane instead of full-size."
    )]
    size_percent: Option<i64>,

    #[arg(long, help = WIDTH_HELP)]
    width: Option<i64>,

    #[arg(long, help = HEIGHT_HELP)]
    height: Option<i64>,

    #[arg(
        long,
        help = "Solid background color (#rrggbb) for the overlay pane, independent of the session's own."
    )]
    background_color: Option<String>,

    #[arg(
        long,
        help = "Scope the overlay to ONE split pane (primary/left/top or split/right/bottom), leaving the sibling \
                pane live and visible; omit for the session-wide overlay. A pane overlay is always full-pane, so \
                this cannot be combined with --size-percent."
    )]
    pane: Option<String>,

    #[command(flatten)]
    target: TargetOptions,

    #[command(flatten)]
    options: ClientOptions,
}

/// A well-formed `--url` whose host is not one of the exact loopback hosts (upstream checks the host only for a URL
/// `HtmlSource.webURL` accepts, leaving a malformed one for the server to name).
fn names_another_host(url: &str) -> bool {
    html::web_url(url).is_some() && !html::is_loopback(url)
}

impl Open {
    /// What the overlay shows: exactly one of COMMAND, `--html` and `--url`, and the options that belong to it.
    fn validate_content(&self) -> Result<(), &'static str> {
        let program = self.command.is_some();
        let sources = [program, self.html.is_some(), self.url.is_some()];
        if sources.iter().filter(|given| **given).count() != 1 {
            return Err("provide exactly one of COMMAND, --html or --url");
        }
        if !program && (self.wait || (self.url.is_some() && self.block)) {
            return Err("a page takes no --wait, and a --url page no --block");
        }
        let refusal = if self.navigation && program {
            "--navigation requires --html or --url"
        } else if self.javascript && program {
            "--js requires --html or --url"
        } else {
            return self.validate_page_options();
        };
        Err(refusal)
    }

    /// The options of one kind of page only.
    fn validate_page_options(&self) -> Result<(), &'static str> {
        let refusal = if self.chromeless && self.command.is_some() {
            "--chromeless requires --html or a loopback --url"
        } else if self.chromeless && self.navigation {
            "--chromeless cannot be combined with --navigation"
        } else if self.chromeless && self.browse {
            "--chromeless cannot be combined with --browse"
        } else if self.chromeless && self.url.as_deref().is_some_and(names_another_host) {
            "--chromeless with --url requires a loopback host: 127.0.0.1, ::1 or localhost"
        } else if self.persistent && self.url.is_none() {
            "--persistent requires --url"
        } else if self.browse && self.url.is_none() {
            "--browse requires --url"
        } else if self.url.is_some() && self.cwd.is_some() {
            "--cwd cannot be combined with --url"
        } else {
            return Ok(());
        };
        Err(refusal)
    }

    /// The `--block` poll of a program (upstream's `resultRequest(id:)`): by the globally unique id the open returned,
    /// so no window scope, and with `--pane`, without which a pane overlay's poll would read the session-wide slot.
    pub fn result_request(&self, id: &str) -> Request {
        Request {
            cmd: Command::Session(SessionCmd::OverlayResult),
            target: Some(id.to_owned()),
            args: self.pane.clone().map(|pane| Args {
                pane: Some(pane),
                ..Args::default()
            }),
        }
    }

    /// Opens the program and waits for it: exits with its status, printing only the server's line under `--json`.
    pub fn block_program(&self, io: &mut dyn ModalIo) -> Result<(), Failure> {
        let json = self.options.basic.json;
        let opened = io.send(&self.make_request())?;
        let id = control_result(&opened).and_then(|result| result.id);
        let Some(id) = id.filter(|_| opened.response.ok) else {
            print_reply(io, &opened, json);
            return Err(Failure::Exit(ExitCode::FAILURE));
        };
        let poll = self.result_request(&id);
        loop {
            let polled = io.send(&poll)?;
            if polled.response.ok {
                if json {
                    io.output(&polled.line());
                }
                // a successful result must carry the status; its absence is a protocol violation, not success
                let Some(code) = control_result(&polled).and_then(|result| result.exit_code) else {
                    io.error_output("error: result missing exit code");
                    return Err(Failure::Exit(ExitCode::FAILURE));
                };
                return match exit_byte(code) {
                    0 => Ok(()),
                    code => Err(Failure::Exit(ExitCode::from(code))),
                };
            }
            if polled.response.error.as_deref() == Some(STILL_RUNNING) {
                io.sleep(PROGRAM_POLL_DELAY);
                continue;
            }
            print_reply(io, &polled, json);
            return Err(Failure::Exit(ExitCode::FAILURE));
        }
    }
}

/// Prints a reply as the default run does: the server's line with `json`, else the summary, a refusal on stderr.
fn print_reply(io: &mut dyn ModalIo, reply: &Reply, json: bool) {
    if json {
        io.output(&reply.line());
    } else if reply.response.ok {
        io.output(&output::format_response(&reply.response, false));
    } else {
        io.error_output(&output::format_response(&reply.response, false));
    }
}

impl RequestCommand for Open {
    fn options(&self) -> &BasicOptions {
        &self.options.basic
    }

    // a page companion's open prints the new row's id, as `session new` does
    fn echoes_result_id(&self) -> bool {
        self.companion
    }

    // the mutually exclusive combinations and a malformed color are refused before any connection
    fn validate(&self) -> Result<(), String> {
        if self.companion && self.html.is_none() && self.url.is_none() {
            return Err("--companion requires --html or --url".to_owned());
        }
        if self.companion
            && (self.pane.is_some() || self.size_percent.is_some() || self.width.is_some() || self.height.is_some())
        {
            return Err("--companion is full size: it takes no --pane, --size-percent, --width or --height".to_owned());
        }
        if self.block && self.wait {
            return Err("--block cannot be combined with --wait".to_owned());
        }
        self.validate_content().map_err(str::to_owned)?;
        if self
            .background_color
            .as_deref()
            .is_some_and(|color| !is_valid_color_hex(color))
        {
            return Err("background-color must be a #rrggbb hex value".to_owned());
        }
        validate_overlay_pane(self.pane.as_deref())?;
        let points = self.width.is_some() || self.height.is_some();
        if self.pane.is_some() && self.size_percent.is_some() {
            return Err("--pane cannot be combined with --size-percent (pane overlays are always full)".to_owned());
        }
        if self.pane.is_some() && points {
            return Err("--pane cannot be combined with --width/--height (pane overlays are always full)".to_owned());
        }
        validate_size_percent(self.size_percent)?;
        validate_points(self.width, self.height)?;
        if points && self.size_percent.is_none() {
            return Err("--width/--height need --size-percent".to_owned());
        }
        Ok(())
    }

    /// The open request; `--block` changes the run, not the request, and `--companion` sends the same arguments as
    /// `session.companion.page`. A page's file and `--cwd` are made absolute against the caller's directory, since the
    /// app never learns it (a companion's as [`overlay_page::companion_path`] says); a program's `--cwd` goes as typed.
    fn make_request(&self) -> Request {
        let page = self.html.is_some();
        let absolute = if self.companion {
            overlay_page::companion_path
        } else {
            overlay_page::absolute_path
        };
        let args = Args {
            cwd: if page {
                self.cwd.as_deref().map(absolute)
            } else {
                self.cwd.clone()
            },
            command: self.command.clone(),
            wait: flag(self.wait),
            size_percent: self.size_percent,
            follow: flag(self.follow),
            pane: self.pane.clone(),
            width: self.width,
            height: self.height,
            color: self.background_color.clone(),
            html: self.html.as_deref().map(absolute),
            navigation: flag(self.navigation),
            url: self.url.clone(),
            javascript: flag(self.javascript),
            chromeless: flag(self.chromeless),
            persistent: flag(self.persistent),
            browse: flag(self.browse),
            caller_drive: if self.companion && page {
                overlay_page::companion_drive([self.html.as_deref(), self.cwd.as_deref()])
            } else {
                None
            },
            ..Args::default()
        };
        let cmd = if self.companion {
            SessionCmd::CompanionPage
        } else {
            SessionCmd::OverlayOpen
        };
        session_request(cmd, Some(self.target.target.clone()), Some(args), &self.options)
    }

    fn invoke(&self, env: &dyn Env) -> Result<(), Failure> {
        if self.companion && !self.block {
            return crate::companions::run(self, env);
        }
        if !self.block {
            return self.default_run(env);
        }
        let mut io = LiveIo {
            socket: self.options.basic.socket_path(env),
        };
        if self.html.is_some() {
            let mut runner = PageRunner {
                json: self.options.basic.json,
                io: &mut io,
            };
            return runner.block(&self.make_request());
        }
        self.block_program(&mut io)
    }
}

// MARK: - close, resize

/// `ngtermctl session overlay close`.
#[derive(Debug, clap::Args)]
#[command(about = "Close the overlay terminal (destroys it; for one shown on another Mac, requests its cancel).")]
pub struct Close {
    #[arg(
        long,
        help = "Close that split pane's overlay (primary/left/top or split/right/bottom); omit for the session-wide \
                overlay."
    )]
    pane: Option<String>,

    #[command(flatten)]
    target: TargetOptions,

    #[command(flatten)]
    options: ClientOptions,
}

/// `{"pane": pane}` when a pane is named, else no args.
fn pane_args(pane: Option<&String>) -> Option<Args> {
    pane.map(|pane| Args {
        pane: Some(pane.clone()),
        ..Args::default()
    })
}

impl RequestCommand for Close {
    fn options(&self) -> &BasicOptions {
        &self.options.basic
    }

    fn validate(&self) -> Result<(), String> {
        validate_overlay_pane(self.pane.as_deref())
    }

    fn make_request(&self) -> Request {
        session_request(
            SessionCmd::OverlayClose,
            Some(self.target.target.clone()),
            pane_args(self.pane.as_ref()),
            &self.options,
        )
    }
}

/// `--width`'s help on open and resize (upstream's `Overlay.widthHelp`).
const WIDTH_HELP: &str =
    "Panel content width in points, in place of --size-percent's width (at most 80% of the session).";
/// `--height`'s help on open and resize (upstream's `Overlay.heightHelp`).
const HEIGHT_HELP: &str = "Panel content height in points, in place of --size-percent's height (at most 80% of the \
                           session); a page's title strip is added on top.";

/// Refuses a `--width`/`--height` that is not positive (upstream's `Overlay.validatePoints`).
fn validate_points(width: Option<i64>, height: Option<i64>) -> Result<(), String> {
    if [width, height].into_iter().flatten().any(|axis| axis <= 0) {
        return Err("--width/--height must be positive".to_owned());
    }
    Ok(())
}

/// `ngtermctl session overlay resize`: `--size-percent` with or without point sizes, point sizes alone, or `--full`.
#[derive(Debug, clap::Args)]
#[command(
    about = "Resize an open overlay: floating at a percent or in points, or back to full-pane.",
    long_about = "--size-percent re-sizes from that percent, keeping only the --width/--height given with it. \
                  --width/--height alone change those axes of an overlay that is already floating."
)]
pub struct Resize {
    #[arg(long, help = "Resize to a floating, framed panel at PERCENT (1-100) of the pane.")]
    size_percent: Option<i64>,

    #[arg(long, help = WIDTH_HELP)]
    width: Option<i64>,

    #[arg(long, help = HEIGHT_HELP)]
    height: Option<i64>,

    #[arg(long, help = "Resize to full-pane (translucent, hides the session).")]
    full: bool,

    #[command(flatten)]
    target: TargetOptions,

    #[command(flatten)]
    options: ClientOptions,
}

impl RequestCommand for Resize {
    fn options(&self) -> &BasicOptions {
        &self.options.basic
    }

    // the dispatcher re-checks the same rules; here they are a clean usage error before any connection
    fn validate(&self) -> Result<(), String> {
        let points = self.width.is_some() || self.height.is_some();
        if self.full && self.size_percent.is_some() {
            return Err("--full cannot be combined with --size-percent".to_owned());
        }
        if self.full && points {
            return Err("--full cannot be combined with --width/--height".to_owned());
        }
        if !self.full && self.size_percent.is_none() && !points {
            return Err("provide --size-percent PERCENT, --width/--height POINTS or --full".to_owned());
        }
        validate_size_percent(self.size_percent)?;
        validate_points(self.width, self.height)
    }

    fn make_request(&self) -> Request {
        let args = Args {
            size_percent: self.size_percent,
            full: flag(self.full),
            width: self.width,
            height: self.height,
            ..Args::default()
        };
        session_request(
            SessionCmd::OverlayResize,
            Some(self.target.target.clone()),
            Some(args),
            &self.options,
        )
    }
}

// MARK: - hide, show, toggle, page-serve

/// Upstream's `Session.Overlay.visibilityScope`, a literal so the discussions can `concat!` it.
macro_rules! visibility_scope {
    () => {
        "Without --pane this acts on every overlay of the session: the session-wide one and both pane overlays."
    };
}

/// Declares an overlay visibility subcommand (upstream's `Session+OverlayVisibility.swift`) sending `$cmd` to its
/// target, scoped to one pane with `--pane`.
macro_rules! visibility_command {
    ($(#[$doc:meta])* $name:ident, $cmd:expr, $about:literal, $discussion:expr) => {
        $(#[$doc])*
        #[derive(Debug, clap::Args)]
        #[command(about = $about, long_about = $discussion)]
        pub struct $name {
            #[arg(long, help = "Only that split pane's overlay (primary/left/top or split/right/bottom).")]
            pane: Option<String>,

            #[command(flatten)]
            target: TargetOptions,

            #[command(flatten)]
            options: ClientOptions,
        }

        impl RequestCommand for $name {
            fn options(&self) -> &BasicOptions {
                &self.options.basic
            }

            fn validate(&self) -> Result<(), String> {
                validate_overlay_pane(self.pane.as_deref())
            }

            fn make_request(&self) -> Request {
                session_request($cmd, Some(self.target.target.clone()), pane_args(self.pane.as_ref()), &self.options)
            }
        }
    };
}

visibility_command!(
    /// `ngtermctl session overlay hide`.
    Hide,
    SessionCmd::OverlayHide,
    "Hide the session's overlays without closing them.",
    concat!(
        "A hidden overlay keeps running and stays in `tree` (hiddenOverlays); the terminal under it takes input. ",
        visibility_scope!()
    )
);
visibility_command!(
    /// `ngtermctl session overlay show`.
    Show,
    SessionCmd::OverlayShow,
    "Show the session's hidden overlays again.",
    visibility_scope!()
);
visibility_command!(
    /// `ngtermctl session overlay toggle`.
    Toggle,
    SessionCmd::OverlayToggle,
    "Hide the session's overlays if any is shown, otherwise show them all.",
    concat!("Prints the state left behind, hidden or shown. ", visibility_scope!())
);

/// `ngtermctl session overlay page-serve TOKEN` (upstream's `OverlayPageServe.swift`): claims a handed-over page by
/// its token and then carries the page's channel between stdin/stdout and the app untouched.
#[derive(Debug, clap::Args)]
#[command(
    name = "page-serve",
    about = "Carry a handed-over page's channel between stdin/stdout and this app.",
    long_about = "Run on this Mac by the ngterm on another Mac, over ssh, when it shows one of this app's pages: it \
                  claims the page with its single-use token and then copies the channel both ways untouched, so the \
                  page's files and bridge requests are served by this app. It is not meant to be run by hand. A \
                  refused token exits 1 with the reason on stderr."
)]
pub struct PageServe {
    #[arg(help = "The token the other Mac was handed with the page.")]
    token: String,

    #[command(flatten)]
    options: BasicOptions,
}

impl RequestCommand for PageServe {
    fn options(&self) -> &BasicOptions {
        &self.options
    }

    fn make_request(&self) -> Request {
        Request {
            target: Some(self.token.clone()),
            ..Request::new(Command::Session(SessionCmd::OverlayPageServe))
        }
    }

    /// Claims the page and bridges its channel until either side ends (upstream's `PageServe.run`): a refused claim
    /// prints `error: <reason>` and exits 1, having written nothing to stdout.
    fn invoke(&self, env: &dyn Env) -> Result<(), Failure> {
        let socket = crate::client::connect(&self.options.socket_path(env))?;
        let bridge = crate::stream_bridge::StreamBridge::over_stdio(socket).map_err(Failure::Local)?;
        if let Err(error) = bridge.open(&self.make_request()) {
            eprintln!("error: {error}");
            return Err(Failure::Exit(ExitCode::FAILURE));
        }
        bridge.pump();
        Ok(())
    }
}

/// `ngtermctl session overlay page-attach`: one attachment of a page companion for another Mac
/// (`agtermctlKit/OverlayPageAttach.swift`).
#[derive(Debug, clap::Args)]
#[command(
    name = "page-attach",
    about = "Hand out one attachment of a page companion to another Mac.",
    long_about = "Run on this Mac by the ngterm on another Mac, over ssh, when it shows one of this app's page \
                  companions: the reply carries the page and a single-use token its `page-serve` claims. It is not \
                  meant to be run by hand."
)]
pub struct PageAttach {
    #[command(flatten)]
    target: TargetOptions,

    #[command(flatten)]
    options: ClientOptions,
}

impl RequestCommand for PageAttach {
    fn options(&self) -> &BasicOptions {
        &self.options.basic
    }

    fn make_request(&self) -> Request {
        session_request(
            SessionCmd::CompanionPageAttach,
            Some(self.target.target.clone()),
            None,
            &self.options,
        )
    }
}

// MARK: - result, copy, text

/// `ngtermctl session overlay result`: a program's `exit N`, or with `--page` a page's outcome, printed and exited as
/// a `--block` page prints and exits.
#[derive(Debug, clap::Args)]
#[command(about = "Print the overlay program's exit status, or with --page an HTML page's outcome.")]
pub struct ResultCommand {
    #[arg(
        long,
        help = "Read that split pane's overlay status (primary/left/top or split/right/bottom); omit for the \
                session-wide overlay."
    )]
    pane: Option<String>,

    #[arg(
        long,
        help = "Read the outcome of the HTML page with this id, as a --block open prints it."
    )]
    page: Option<String>,

    #[command(flatten)]
    target: TargetOptions,

    #[command(flatten)]
    options: ClientOptions,
}

impl RequestCommand for ResultCommand {
    fn options(&self) -> &BasicOptions {
        &self.options.basic
    }

    fn validate(&self) -> Result<(), String> {
        validate_overlay_pane(self.pane.as_deref())?;
        if self.page.is_some() && self.pane.is_some() {
            return Err("--page cannot be combined with --pane".to_owned());
        }
        Ok(())
    }

    /// A page is read by its globally unique id alone: no session, no window.
    fn make_request(&self) -> Request {
        if let Some(page) = &self.page {
            return overlay_page::page_result_request(page);
        }
        session_request(
            SessionCmd::OverlayResult,
            Some(self.target.target.clone()),
            pane_args(self.pane.as_ref()),
            &self.options,
        )
    }

    fn invoke(&self, env: &dyn Env) -> Result<(), Failure> {
        let Some(page) = &self.page else {
            return self.default_run(env);
        };
        let mut io = LiveIo {
            socket: self.options.basic.socket_path(env),
        };
        PageRunner {
            json: self.options.basic.json,
            io: &mut io,
        }
        .read(page)
    }
}

/// `ngtermctl session overlay copy`.
#[derive(Debug, clap::Args)]
#[command(about = "Print the selection made INSIDE the overlay (session copy reads the pane underneath).")]
pub struct Copy {
    #[arg(
        long,
        help = "Read that split pane's overlay (primary/left/top or split/right/bottom); omit for the session-wide \
                overlay."
    )]
    pane: Option<String>,

    #[command(flatten)]
    target: TargetOptions,

    #[command(flatten)]
    options: ClientOptions,
}

impl RequestCommand for Copy {
    fn options(&self) -> &BasicOptions {
        &self.options.basic
    }

    fn validate(&self) -> Result<(), String> {
        validate_overlay_pane(self.pane.as_deref())
    }

    fn make_request(&self) -> Request {
        session_request(
            SessionCmd::OverlayCopy,
            Some(self.target.target.clone()),
            pane_args(self.pane.as_ref()),
            &self.options,
        )
    }
}

/// `ngtermctl session overlay text`.
#[derive(Debug, clap::Args)]
#[command(about = "Print the overlay's terminal buffer as plain text (a TUI's drawn screen, wrapped as rendered).")]
pub struct Text {
    #[arg(long, help = "Read the full screen + scrollback instead of just the visible screen.")]
    all: bool,

    #[arg(long, help = "Keep only the last N lines of the full buffer.")]
    lines: Option<i64>,

    #[arg(
        long,
        help = "Read that split pane's overlay (primary/left/top or split/right/bottom); omit for the session-wide \
                overlay."
    )]
    pane: Option<String>,

    #[command(flatten)]
    target: TargetOptions,

    #[command(flatten)]
    options: ClientOptions,
}

impl RequestCommand for Text {
    fn options(&self) -> &BasicOptions {
        &self.options.basic
    }

    // the dispatcher's order, so the CLI and the socket refuse the same call the same way
    fn validate(&self) -> Result<(), String> {
        if self.all && self.lines.is_some() {
            return Err("use either --all or --lines, not both".to_owned());
        }
        if self.lines.is_some_and(|lines| lines <= 0) {
            return Err("--lines must be greater than 0".to_owned());
        }
        validate_overlay_pane(self.pane.as_deref())
    }

    fn make_request(&self) -> Request {
        let args = Args {
            pane: self.pane.clone(),
            all: flag(self.all),
            lines: self.lines,
            ..Args::default()
        };
        // upstream always builds the bag, so a bare `session overlay text` sends `"args": {}`
        session_request(
            SessionCmd::OverlayText,
            Some(self.target.target.clone()),
            Some(args),
            &self.options,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use clap::Parser;
    use serde_json::{Value, json};

    use super::*;
    use crate::modal::tests::{FakeIo, answer, reply};
    use crate::{Command as Cli, Ngtermctl};

    /// `ngtermctl session overlay <argv>`.
    fn command_line(argv: &[&str]) -> Vec<OsString> {
        ["ngtermctl", "session", "overlay"]
            .iter()
            .chain(argv)
            .map(OsString::from)
            .collect()
    }

    /// `ngtermctl session overlay <argv>` parsed and validated: the request it builds as JSON, or the refusal.
    fn outcome(argv: &[&str]) -> Result<Value, String> {
        let parsed = Ngtermctl::try_parse_from(command_line(argv)).map_err(|error| error.to_string())?;
        let Cli::Session(group) = parsed.command else {
            panic!("not a session command")
        };
        let command = group.command();
        command.validate()?;
        Ok(serde_json::to_value(command.make_request()).unwrap())
    }

    fn request(argv: &[&str]) -> Value {
        outcome(argv).unwrap()
    }

    /// A `/`-spelled absolute path as `absolute_path` sends it on this OS: as it is on Unix, on the current
    /// directory's drive with `\` separators on Windows.
    fn native(path: &str) -> String {
        if cfg!(windows) {
            let cwd = std::env::current_dir().unwrap();
            let drive = cwd
                .components()
                .next()
                .unwrap()
                .as_os_str()
                .to_string_lossy()
                .into_owned();
            format!("{drive}{}", path.replace('/', "\\"))
        } else {
            path.to_owned()
        }
    }

    fn args(argv: &[&str]) -> Value {
        request(argv)["args"].clone()
    }

    fn refusal(argv: &[&str]) -> String {
        outcome(argv).unwrap_err()
    }

    /// Whether `argv` fails to parse or validate.
    fn rejects(argv: &[&str]) -> bool {
        outcome(argv).is_err()
    }

    /// The overlay group alone, to reach a parsed `open`.
    #[derive(Debug, Parser)]
    struct OverlayLine {
        #[command(subcommand)]
        command: OverlayCommand,
    }

    /// The parsed `open <argv>`.
    fn open(argv: &[&str]) -> Box<Open> {
        let line = ["overlay", "open"].iter().chain(argv);
        match OverlayLine::try_parse_from(line).unwrap().command {
            OverlayCommand::Open(open) => open,
            other => panic!("not overlay open: {other:?}"),
        }
    }

    /// Upstream's `overlayPageAttachSendsTheAttachCommandForTheTarget`.
    #[test]
    fn page_attach_sends_the_attach_command_for_the_target() {
        assert_eq!(
            request(&["page-attach", "--target", "pg", "--window", "win"]),
            json!({"cmd": "session.companion.page.attach", "target": "pg", "args": {"window": "win"}})
        );
    }

    /// Upstream's `overlayOpenWithCompanionSendsThePageCompanionCommand`.
    #[test]
    fn open_with_companion_sends_the_page_companion_command() {
        let html = "/tmp/p.html";
        // a Windows caller adds its drive, for a local row to put the driveless path on
        let mut args = json!({"html": html});
        if let Some(drive) = overlay_page::companion_drive([Some(html)]) {
            args["callerDrive"] = json!(drive);
        }
        assert_eq!(
            request(&["open", "--html", html, "--companion", "--target", "p1"]),
            json!({"cmd": "session.companion.page", "target": "p1", "args": args})
        );
        assert!(args.get("callerDrive").is_some() == cfg!(windows), "{args}");
        assert!(open(&["--html", html, "--companion"]).echoes_result_id());
        assert!(!open(&["--html", html]).echoes_result_id());
    }

    /// Upstream's `anHtmlCompanionMayBlockOnItsPageAndAURLOneMayNot`.
    #[test]
    fn an_html_companion_may_block_on_its_page_and_a_url_one_may_not() {
        assert_eq!(
            request(&["open", "--html", "/tmp/p.html", "--companion", "--block"])["cmd"],
            json!("session.companion.page")
        );
        assert!(rejects(&[
            "open",
            "--url",
            "http://localhost:1",
            "--companion",
            "--block"
        ]));
    }

    /// Upstream's `overlayOpenWithCompanionRefusesAnythingButFullSize` and `overlayOpenWithCompanionNeedsAPage`.
    #[test]
    fn open_with_companion_refuses_anything_but_full_size_and_needs_a_page() {
        let full = "--companion is full size: it takes no --pane, --size-percent, --width or --height";
        for extra in [
            ["--pane", "right"],
            ["--size-percent", "50"],
            ["--width", "300"],
            ["--height", "300"],
        ] {
            let argv = [&["open", "--html", "/tmp/p.html", "--companion"][..], &extra[..]].concat();
            assert_eq!(refusal(&argv), full, "{extra:?}");
        }
        assert_eq!(
            refusal(&["open", "top", "--companion"]),
            "--companion requires --html or --url"
        );
    }

    #[test]
    fn open_sends_the_program_and_its_options() {
        assert_eq!(
            request(&["open", "revdiff", "--cwd", "/b", "--target", "9f3c"]),
            json!({"cmd": "session.overlay.open", "target": "9f3c", "args": {"cwd": "/b", "command": "revdiff"}})
        );
        assert_eq!(
            request(&["open", "revdiff"]),
            json!({"cmd": "session.overlay.open", "target": "active", "args": {"command": "revdiff"}})
        );
        assert_eq!(
            args(&["open", "revdiff", "--wait"]),
            json!({"command": "revdiff", "wait": true})
        );
        assert_eq!(
            args(&["open", "htop", "--size-percent", "70"]),
            json!({"command": "htop", "sizePercent": 70})
        );
        // --background-color rides the shared `color` field
        assert_eq!(
            args(&["open", "revdiff", "--background-color", "#2a1a3a"]),
            json!({"command": "revdiff", "color": "#2a1a3a"})
        );
        assert_eq!(
            args(&["open", "revdiff", "--follow"]),
            json!({"command": "revdiff", "follow": true})
        );
        let every = [
            "open",
            "revdiff",
            "--pane",
            "right",
            "--cwd",
            "/b",
            "--wait",
            "--follow",
            "--background-color",
            "#2a1a3a",
            "--window",
            "w1",
        ];
        assert_eq!(
            args(&every),
            json!({"cwd": "/b", "command": "revdiff", "wait": true, "follow": true, "pane": "right",
                   "color": "#2a1a3a", "window": "w1"})
        );
        for pane in ["left", "right", "top", "bottom", "primary", "split"] {
            assert_eq!(args(&["open", "revdiff", "--pane", pane])["pane"], pane);
        }
    }

    #[test]
    fn block_changes_the_run_not_the_request() {
        assert_eq!(
            request(&[
                "open",
                "htop",
                "--follow",
                "--block",
                "--size-percent",
                "60",
                "--target",
                "9f3c"
            ]),
            json!({"cmd": "session.overlay.open", "target": "9f3c",
                   "args": {"command": "htop", "sizePercent": 60, "follow": true}})
        );
        assert_eq!(args(&["open", "revdiff", "--block"]), json!({"command": "revdiff"}));
    }

    #[test]
    fn the_block_poll_carries_the_pane_and_no_window() {
        let poll = open(&["revdiff", "--block", "--pane", "right", "--window", "w1"]).result_request("9f3c");
        assert_eq!(
            serde_json::to_value(poll).unwrap(),
            json!({"cmd": "session.overlay.result", "target": "9f3c", "args": {"pane": "right"}})
        );
        let poll = open(&["revdiff", "--block"]).result_request("9f3c");
        assert_eq!(
            serde_json::to_value(poll).unwrap(),
            json!({"cmd": "session.overlay.result", "target": "9f3c"})
        );
    }

    #[test]
    fn a_page_is_sent_absolute_with_its_grant_and_a_url_untouched() {
        let page = [
            "open",
            "--html",
            "/tmp/a/../a/./r.html",
            "--cwd",
            "/tmp/a/",
            "--pane",
            "right",
            "--background-color",
            "#102030",
            "--target",
            "s",
        ];
        assert_eq!(
            args(&page),
            json!({"html": native("/tmp/a/r.html"), "cwd": native("/tmp/a"), "pane": "right", "color": "#102030"})
        );
        assert_eq!(
            args(&["open", "--html", "/tmp/r.html", "--chromeless", "--js"]),
            json!({"html": native("/tmp/r.html"), "chromeless": true, "javascript": true})
        );
        assert_eq!(
            args(&["open", "--html", "/tmp/r.html", "--navigation", "--js"]),
            json!({"html": native("/tmp/r.html"), "navigation": true, "javascript": true})
        );
        let here = std::env::current_dir().unwrap();
        assert_eq!(
            args(&["open", "--html", "out/r.html", "--cwd", "out"]),
            json!({"html": here.join("out").join("r.html"), "cwd": here.join("out")})
        );
        assert_eq!(
            args(&["open", "--url", "http://localhost:5173/a/../b", "--navigation", "--js"]),
            json!({"url": "http://localhost:5173/a/../b", "navigation": true, "javascript": true})
        );
        let url = [
            "open",
            "--url",
            "http://localhost:5173/",
            "--browse",
            "--js",
            "--navigation",
            "--persistent",
        ];
        assert_eq!(
            args(&url),
            json!({"url": "http://localhost:5173/", "browse": true, "javascript": true, "navigation": true,
                   "persistent": true})
        );
        // upstream's urlOpenSendsChromelessForALoopbackHost
        for url in ["http://127.0.0.1:8792/", "http://[::1]:8792/", "http://localhost:5173/"] {
            assert_eq!(
                args(&["open", "--url", url, "--chromeless", "--js", "--persistent"]),
                json!({"url": url, "chromeless": true, "javascript": true, "persistent": true}),
                "{url}"
            );
        }
        // upstream's aMalformedChromelessUrlIsLeftForTheServerToName
        assert_eq!(
            args(&["open", "--url", "localhost:5173", "--chromeless"]),
            json!({"url": "localhost:5173", "chromeless": true})
        );
        // a program's --cwd goes as typed
        assert_eq!(
            args(&["open", "revdiff", "--cwd", "repo"]),
            json!({"cwd": "repo", "command": "revdiff"})
        );
        // a page may block, a URL may not
        assert_eq!(
            args(&["open", "--html", "/tmp/r.html", "--block"]),
            json!({"html": native("/tmp/r.html")})
        );
    }

    #[test]
    fn open_refuses_a_missing_or_conflicting_content() {
        let cases: [&[&str]; 22] = [
            &["open"],
            &["open", "revdiff", "--html", "/tmp/r.html"],
            &["open", "--html", "/tmp/r.html", "--wait"],
            &["open", "--html", "/tmp/r.html", "--block", "--wait"],
            &["open", "revdiff", "--navigation"],
            &["open", "revdiff", "--js"],
            &["open", "revdiff", "--chromeless"],
            &["open", "--url", "http://example.com/", "--chromeless"],
            &["open", "--url", "http://127.0.0.2:8792/", "--chromeless"],
            &["open", "--url", "http://localhost:5173/", "--chromeless", "--browse"],
            &[
                "open",
                "--url",
                "http://localhost:5173/",
                "--chromeless",
                "--navigation",
            ],
            &["open", "--html", "/tmp/r.html", "--chromeless", "--navigation"],
            &["open", "revdiff", "--persistent"],
            &["open", "--html", "/tmp/r.html", "--persistent"],
            &["open", "revdiff", "--browse"],
            &["open", "--html", "/tmp/r.html", "--browse"],
            &["open", "revdiff", "--url", "http://localhost:5173/"],
            &["open", "--html", "/tmp/r.html", "--url", "http://localhost:5173/"],
            &["open", "--url", "http://localhost:5173/", "--wait"],
            &["open", "--url", "http://localhost:5173/", "--block"],
            &["open", "--url", "http://localhost:5173/", "--cwd", "/tmp"],
            &["open", "cmd", "--background-color", "purple"],
        ];
        for argv in cases {
            assert!(rejects(argv), "{argv:?}");
        }
    }

    #[test]
    fn open_refusals_name_what_is_wrong() {
        let cases: [(&[&str], &str); 16] = [
            (
                &["open", "revdiff", "--chromeless"],
                "--chromeless requires --html or a loopback --url",
            ),
            (
                &[
                    "open",
                    "--url",
                    "http://localhost:5173/",
                    "--chromeless",
                    "--navigation",
                ],
                "--chromeless cannot be combined with --navigation",
            ),
            (
                &["open", "--url", "http://localhost:5173/", "--chromeless", "--browse"],
                "--chromeless cannot be combined with --browse",
            ),
            (
                &["open", "--url", "http://example.com/", "--chromeless"],
                "--chromeless with --url requires a loopback host: 127.0.0.1, ::1 or localhost",
            ),
            (
                &["open", "--url", "http://127.0.0.2:8792/", "--chromeless"],
                "--chromeless with --url requires a loopback host: 127.0.0.1, ::1 or localhost",
            ),
            (
                &["open", "--url", "http://127.1/", "--chromeless"],
                "--chromeless with --url requires a loopback host: 127.0.0.1, ::1 or localhost",
            ),
            (
                &["open", "cmd", "--block", "--wait"],
                "--block cannot be combined with --wait",
            ),
            (
                &["open", "cmd", "--pane", "left", "--block", "--wait"],
                "--block cannot be combined with --wait",
            ),
            (&["open"], "provide exactly one of COMMAND, --html or --url"),
            (
                &["open", "--url", "http://x/", "--block"],
                "a page takes no --wait, and a --url page no --block",
            ),
            (
                &["open", "cmd", "--size-percent", "0"],
                "--size-percent must be between 1 and 100",
            ),
            (
                &["open", "cmd", "--size-percent", "150"],
                "--size-percent must be between 1 and 100",
            ),
            (
                &["open", "cmd", "--background-color", "purple"],
                "background-color must be a #rrggbb hex value",
            ),
            (&["open", "cmd", "--pane", "scratch"], "--pane must be left or right"),
            (&["open", "cmd", "--pane", "middle"], "--pane must be left or right"),
            (
                &["open", "cmd", "--pane", "left", "--size-percent", "150"],
                "--pane cannot be combined with --size-percent (pane overlays are always full)",
            ),
        ];
        for (argv, message) in cases {
            assert_eq!(refusal(argv), message, "{argv:?}");
        }
    }

    #[test]
    fn the_pane_commands_refuse_the_scratch() {
        for argv in [
            &["close", "--pane", "scratch"][..],
            &["result", "--pane", "scratch"],
            &["copy", "--pane", "scratch"],
            &["text", "--pane", "scratch"],
            &["reload", "--pane", "middle"],
            &["submit", "--value", "x", "--pane", "middle"],
            &["navigate", "back", "--pane", "scratch"],
        ] {
            assert_eq!(refusal(argv), "--pane must be left or right", "{argv:?}");
        }
    }

    #[test]
    fn close_copy_and_result_send_a_pane_only_when_named() {
        assert_eq!(
            request(&["close"]),
            json!({"cmd": "session.overlay.close", "target": "active"})
        );
        assert_eq!(args(&["close", "--pane", "left"]), json!({"pane": "left"}));
        assert_eq!(
            request(&["copy", "--pane", "right", "--target", "9f3c"]),
            json!({"cmd": "session.overlay.copy", "target": "9f3c", "args": {"pane": "right"}})
        );
        assert_eq!(
            request(&["copy"]),
            json!({"cmd": "session.overlay.copy", "target": "active"})
        );
        assert_eq!(
            request(&["result", "--target", "9f3c"]),
            json!({"cmd": "session.overlay.result", "target": "9f3c"})
        );
        assert_eq!(
            args(&["result", "--pane", "right", "--window", "w1"]),
            json!({"pane": "right", "window": "w1"})
        );
    }

    #[test]
    fn a_page_result_reads_the_page_alone() {
        assert_eq!(
            request(&["result", "--page", "P1", "--target", "s", "--window", "w1"]),
            json!({"cmd": "session.overlay.result", "args": {"page": "P1"}})
        );
        assert_eq!(
            refusal(&["result", "--page", "P1", "--pane", "left"]),
            "--page cannot be combined with --pane"
        );
    }

    #[test]
    fn resize_takes_exactly_one_size() {
        assert_eq!(
            request(&["resize", "--size-percent", "40", "--target", "9f3c"]),
            json!({"cmd": "session.overlay.resize", "target": "9f3c", "args": {"sizePercent": 40}})
        );
        assert_eq!(args(&["resize", "--full"]), json!({"full": true}));
        assert_eq!(
            refusal(&["resize"]),
            "provide --size-percent PERCENT, --width/--height POINTS or --full"
        );
        assert_eq!(
            refusal(&["resize", "--full", "--size-percent", "50"]),
            "--full cannot be combined with --size-percent"
        );
        assert_eq!(
            refusal(&["resize", "--size-percent", "0"]),
            "--size-percent must be between 1 and 100"
        );
        assert!(rejects(&["resize", "--full", "--pane", "left"]), "resize has no --pane");
    }

    #[test]
    fn open_and_resize_send_point_sizes() {
        assert_eq!(
            args(&[
                "open",
                "--html",
                "/tmp/r.html",
                "--size-percent",
                "60",
                "--width",
                "700",
                "--height",
                "240"
            ]),
            json!({"sizePercent": 60, "width": 700, "height": 240, "html": native("/tmp/r.html")})
        );
        assert_eq!(
            request(&["resize", "--height", "300", "--target", "s"]),
            json!({"cmd": "session.overlay.resize", "target": "s", "args": {"height": 300}})
        );
        assert_eq!(
            args(&["resize", "--size-percent", "60", "--width", "700"]),
            json!({"sizePercent": 60, "width": 700})
        );
    }

    #[test]
    fn point_sizes_are_refused_before_any_connection() {
        let cases: [(&[&str], &str); 5] = [
            (
                &["open", "--html", "/tmp/r.html", "--height", "240"],
                "--width/--height need --size-percent",
            ),
            (
                &["open", "cat", "--size-percent", "60", "--width", "0"],
                "--width/--height must be positive",
            ),
            (
                &["open", "cat", "--pane", "right", "--height", "240"],
                "--pane cannot be combined with --width/--height (pane overlays are always full)",
            ),
            (
                &["resize", "--full", "--height", "240"],
                "--full cannot be combined with --width/--height",
            ),
            (&["resize", "--width", "0"], "--width/--height must be positive"),
        ];
        for (argv, message) in cases {
            assert_eq!(refusal(argv), message, "{argv:?}");
        }
        assert!(rejects(&["resize", "--width", "-1"]), "a negative size is refused too");
    }

    #[test]
    fn text_carries_its_extent_and_pane() {
        assert_eq!(
            request(&["text", "--all", "--pane", "left", "--target", "9f3c"]),
            json!({"cmd": "session.overlay.text", "target": "9f3c", "args": {"pane": "left", "all": true}})
        );
        assert_eq!(args(&["text", "--lines", "20"]), json!({"lines": 20}));
        assert_eq!(args(&["text"]), json!({}));
        assert_eq!(
            refusal(&["text", "--all", "--lines", "5"]),
            "use either --all or --lines, not both"
        );
        assert_eq!(refusal(&["text", "--lines", "0"]), "--lines must be greater than 0");
    }

    #[test]
    fn page_commands_send_their_step_value_and_pane() {
        assert_eq!(
            request(&["reload", "--pane", "left", "--target", "s"]),
            json!({"cmd": "session.overlay.reload", "target": "s", "args": {"pane": "left"}})
        );
        assert_eq!(args(&["reload", "--current"]), json!({"current": true}));
        assert_eq!(
            request(&["submit", "--value", "", "--pane", "right", "--target", "s"]),
            json!({"cmd": "session.overlay.submit", "target": "s", "args": {"pane": "right", "value": ""}})
        );
        assert!(rejects(&["submit"]), "submit needs --value");
        for step in ["back", "forward", "browser", "finder"] {
            assert_eq!(
                request(&["navigate", step, "--pane", "right"]),
                json!({"cmd": "session.overlay.navigate", "target": "active", "args": {"pane": "right", "to": step}})
            );
        }
        for argv in [&["navigate", "up"][..], &["navigate", "copy"], &["navigate"]] {
            assert!(rejects(argv), "{argv:?}");
        }
        assert_eq!(
            refusal(&["navigate", "up"]),
            "step must be back, forward, browser, or finder"
        );
    }

    /// Upstream's `OverlayPageServeTests`: the token is the request's target, and required.
    #[test]
    fn page_serve_names_the_token_as_its_target() {
        assert_eq!(
            request(&["page-serve", "tok-1"]),
            json!({"cmd": "session.overlay.page.serve", "target": "tok-1"})
        );
        assert!(rejects(&["page-serve"]), "page-serve needs a token");
        assert!(
            rejects(&["page-serve", "tok-1", "--window", "w1"]),
            "page-serve takes no window"
        );
    }

    #[test]
    fn run_job_claims_the_job_it_names() {
        assert_eq!(
            request(&["run-job", "job-id"]),
            json!({"cmd": "session.overlay.job.run", "target": "job-id"})
        );
        assert!(rejects(&["run-job"]));
        assert!(
            rejects(&["run-job", "job-id", "--window", "w1"]),
            "run-job takes no window"
        );
    }

    fn opened(id: &str) -> Reply {
        answer(&json!({"id": id}))
    }

    fn exit_status(outcome: Result<(), Failure>) -> Option<ExitCode> {
        match outcome {
            Ok(()) => None,
            Err(Failure::Exit(code)) => Some(code),
            Err(other) => panic!("unexpected failure: {other:?}"),
        }
    }

    fn is_open(request: &Request) -> bool {
        request.cmd == Command::Session(SessionCmd::OverlayOpen)
    }

    #[test]
    fn a_blocked_program_polls_until_it_ends_and_exits_with_its_status() {
        let mut polls = 0;
        let mut io = FakeIo::new(|request: &Request| {
            if is_open(request) {
                return Ok(opened("abc"));
            }
            polls += 1;
            Ok(if polls == 1 {
                reply(r#"{"ok":false,"error":"overlay still running"}"#)
            } else {
                answer(&json!({"id": "abc", "exitCode": 7}))
            })
        });
        let status = exit_status(open(&["echo", "--block", "--pane", "left"]).block_program(&mut io));
        assert_eq!(status, Some(ExitCode::from(7)));
        assert_eq!(io.delays, vec![PROGRAM_POLL_DELAY]);
        let poll = json!({"cmd": "session.overlay.result", "target": "abc", "args": {"pane": "left"}});
        assert_eq!(io.requests[1..], [poll.clone(), poll]);
        assert!(
            io.lines.is_empty() && io.errors.is_empty(),
            "{:?} {:?}",
            io.lines,
            io.errors
        );
    }

    #[test]
    fn a_program_exiting_zero_succeeds_and_json_prints_the_result_line() {
        let line = r#"{"ok":true,"result":{"id":"abc","exitCode":0}}"#;
        let mut io = FakeIo::new(|request: &Request| Ok(if is_open(request) { opened("abc") } else { reply(line) }));
        assert_eq!(
            exit_status(open(&["true", "--block", "--json"]).block_program(&mut io)),
            None
        );
        assert_eq!(io.lines, vec![line.to_owned()]);
    }

    #[test]
    fn a_result_without_its_status_fails() {
        let mut io = FakeIo::new(|request: &Request| {
            Ok(if is_open(request) {
                opened("abc")
            } else {
                answer(&json!({"id": "abc"}))
            })
        });
        let status = exit_status(open(&["echo", "--block"]).block_program(&mut io));
        assert_eq!(status, Some(ExitCode::FAILURE));
        assert_eq!(io.errors, vec!["error: result missing exit code".to_owned()]);
    }

    #[test]
    fn a_refused_open_or_poll_exits_1_with_the_error() {
        let mut io = FakeIo::new(|_: &Request| Ok(reply(r#"{"ok":false,"error":"overlay already open"}"#)));
        let status = exit_status(open(&["echo", "--block"]).block_program(&mut io));
        assert_eq!(status, Some(ExitCode::FAILURE));
        assert_eq!(io.errors, vec!["error: overlay already open".to_owned()]);
        assert_eq!(io.requests.len(), 1, "nothing is polled after a refused open");

        let mut io = FakeIo::new(|request: &Request| {
            Ok(if is_open(request) {
                opened("abc")
            } else {
                reply(r#"{"ok":false,"error":"no overlay"}"#)
            })
        });
        let status = exit_status(open(&["echo", "--block"]).block_program(&mut io));
        assert_eq!(status, Some(ExitCode::FAILURE));
        assert_eq!(io.errors, vec!["error: no overlay".to_owned()]);
    }

    #[test]
    fn an_exit_status_keeps_its_low_byte() {
        assert_eq!(exit_byte(5), 5);
        assert_eq!(exit_byte(256 + 3), 3);
        assert_eq!(exit_byte(-1), 255);
    }
}
