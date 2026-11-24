//! A session link tapped in a pane, the app's appearance reported to the program in it, and the origin's unseen badge
//! cleared by what the phone types: each as a user meets it in a pane the phone leads.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ngterm_mobile::{CellPoint, PaneColorScheme, PaneRole};

use crate::machine::World;
use crate::phone::{Client, GRID, MACHINE_ID, block_on, enter, poll, screen_text};

/// The color scheme report of a dark (`1`) or light (`2`) appearance, as the screen shows it with its ESC dropped.
const DARK_REPORT: &str = "[?997;1n";
const LIGHT_REPORT: &str = "[?997;2n";

/// A program that asks for the color scheme (`CSI ? 996 n`) and prints every answer it reads within three seconds,
/// so a second answer shows too, before the bar that ends them.
const ASK_SCHEME: &str = r#"stty raw -echo; printf '\033[?996n'; r=$(timeout --foreground 3 cat); stty sane
printf '%s:%s|\n' asked "$r" | tr -d '\033'
"#;

/// A program that turns on mode 2031, says it is ready, and prints the first report it reads.
const AWAIT_FLIP: &str = r#"stty raw -echo; printf '\033[?2031h%s%s\r\n' arm ed; r=$(dd bs=1 count=9 2>/dev/null)
printf '\033[?2031l'; stty sane; printf '%s%s:%s\n' flip ped "$r" | tr -d '\033'
"#;

/// The phone's appearance reaches the program in its pane: a query sent right after a change gets the app's new light
/// or dark, and with mode 2031 on, a flip of the appearance is reported.
pub(crate) fn the_program_gets_the_apps_appearance(client: &mut Client) {
    let pane = client.terminal.clone().expect("a shown pane");
    pane.set_color_scheme(PaneColorScheme::Dark);
    run(client, ASK_SCHEME);
    client.wait_screen(&format!("asked:{DARK_REPORT}|"));

    pane.set_color_scheme(PaneColorScheme::Light);
    run(client, ASK_SCHEME);
    client.wait_screen(&format!("asked:{LIGHT_REPORT}|"));

    run(client, AWAIT_FLIP);
    client.wait_screen("armed");
    pane.set_color_scheme(PaneColorScheme::Dark);
    client.wait_screen(&format!("flipped:{DARK_REPORT}"));
}

/// A key the phone types into the session's main pane clears the badge a notification raised on the origin.
pub(crate) fn typing_clears_the_origins_badge(world: &World, client: &mut Client, session: &str) {
    world
        .headless
        .ctl(&["notify", "e2e badge", "--target", session])
        .expect("notify the session");
    poll("the origin's unseen badge", || {
        (unseen(world, session) > 0).then_some(())
    });
    let pane = client.terminal.clone().expect("a shown pane");
    assert!(pane.key(enter()).expect("press enter"));
    poll("the badge cleared by the phone's key", || {
        (unseen(world, session) == 0).then_some(())
    });
}

/// A key the phone types into the session's split pane clears the badge too: the origin splits the session, the
/// phone shows the split's other pane, and Enter there clears what a notification raised.
pub(crate) fn typing_in_the_split_pane_clears_the_origins_badge(world: &World, client: &mut Client, session: &str) {
    let main = client.pane.clone().expect("the main pane's id");
    world
        .headless
        .ctl(&["session", "split", "on", "--target", session])
        .expect("split the session");
    client.wait("the split's layout", |seen| {
        seen.layout
            .as_ref()
            .is_some_and(|layout| layout.panes.len() == 2 && layout.shown)
    });
    let layout = client.seen.layout.clone().expect("a layout");
    let split = layout
        .panes
        .iter()
        .find(|pane| **pane != main)
        .cloned()
        .unwrap_or_else(|| panic!("no split pane in {layout:?}"));
    client.drain();
    let roles = client.seen.roles.len();
    // a pane shows from the machine's last listing, which the split came after: the app lists again on the tree's
    // change, as this does
    let shown = poll("the split pane shown", || {
        client.list();
        block_on(
            client
                .core
                .show_pane(MACHINE_ID.to_owned(), session.to_owned(), split.clone(), GRID),
        )
        .ok()
    });
    client.wait("the split pane's role", |seen| {
        seen.roles[roles..].iter().any(Option::is_some)
    });
    if client.seen.role != Some(PaneRole::Leader) {
        client.take_lead(session, &split);
    }

    world
        .headless
        .ctl(&["notify", "e2e split badge", "--target", session])
        .expect("notify the session");
    poll("the origin's unseen badge", || {
        (unseen(world, session) > 0).then_some(())
    });
    assert!(shown.key(enter()).expect("press enter in the split pane"));
    poll("the badge cleared by the phone's key in the split pane", || {
        (unseen(world, session) == 0).then_some(())
    });

    client
        .core
        .hide_pane(MACHINE_ID.to_owned(), session.to_owned(), split)
        .expect("hide the split pane");
    world
        .headless
        .ctl(&["session", "split", "off", "--target", session])
        .expect("hide the split");
    client.wait("the split hidden", |seen| {
        seen.layout.as_ref().is_some_and(|layout| !layout.shown)
    });
}

/// A link to another session of the machine, made after the phone's last listing, names that session, so a tap opens
/// it; a link to a session the machine does not list, or to a web page, names none.
pub(crate) fn a_session_link_names_its_session(world: &World, client: &mut Client) {
    let other = world
        .headless
        .ctl(&["session", "new", "--no-select", "--name=e2e-linked"])
        .expect("create the linked session");
    let other = other.trim().to_owned();
    let unknown = "0E2B7C1A-5D4F-4E3B-9A61-7C2D8F0B1A23";
    run(
        client,
        &format!(
            r"printf '\033]8;;%s\007%s\033]8;;\007\n' ngterm://session/{other} go-to-b ngterm://session/{unknown} go-to-x https://example.com go-to-web
"
        ),
    );
    client.wait_screen("go-to-web");

    let target = block_on(session_link(client, "go-to-b")).expect("the link's session");
    assert_eq!(
        target.map(|session| (session.id.to_uppercase(), session.name)),
        Some((other.to_uppercase(), "e2e-linked".to_owned()))
    );
    for label in ["go-to-x", "go-to-web"] {
        let target = block_on(session_link(client, label)).expect("the link's session");
        assert!(target.is_none(), "{label} names {target:?}");
    }
    let plain = block_on(
        client
            .terminal
            .clone()
            .expect("a shown pane")
            .session_link(CellPoint { x: 0, y: 0 }),
    );
    assert!(matches!(plain, Ok(None)), "a cell with no link names {plain:?}");
    world
        .headless
        .ctl(&["session", "close", "--target", &other])
        .expect("close the linked session");
}

/// What the session link on the last printed `label` names.
async fn session_link(
    client: &Client,
    label: &str,
) -> Result<Option<ngterm_mobile::RemoteSession>, ngterm_mobile::PaneInputError> {
    let pane = client.terminal.clone().expect("a shown pane");
    let screen = screen_text(&pane.frame(0).expect("a frame"));
    let row = screen
        .lines()
        .collect::<Vec<_>>()
        .iter()
        .rposition(|line| line.starts_with(label))
        .unwrap_or_else(|| panic!("no printed {label}:\n{screen}"));
    let at = CellPoint {
        x: 1,
        y: u32::try_from(row).expect("a row"),
    };
    pane.session_link(at).await
}

/// Runs `script` in the pane's shell from a file: the typed line has no quote an interactive shell could pair, and the
/// program reads the terminal, not its own text.
fn run(client: &mut Client, script: &str) {
    let pane = client.terminal.clone().expect("a shown pane");
    let line = format!("echo {}|base64 -d>e2e-run.sh;sh e2e-run.sh", STANDARD.encode(script));
    assert!(pane.type_text(line).expect("type"));
    assert!(pane.key(enter()).expect("press enter"));
}

/// The session's unseen count on the origin.
fn unseen(world: &World, session: &str) -> u64 {
    let output = world.headless.ctl(&["tree", "--json"]).expect("the origin's tree");
    let reply: serde_json::Value = serde_json::from_str(&output).expect("tree's json");
    reply["result"]["tree"]["workspaces"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|workspace| workspace["sessions"].as_array().into_iter().flatten())
        .find(|found| found["id"].as_str().is_some_and(|id| id.eq_ignore_ascii_case(session)))
        .and_then(|found| found["unseen"].as_u64())
        .unwrap_or(0)
}
