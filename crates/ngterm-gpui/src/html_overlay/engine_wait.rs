//! What a page shows in its own area while the engine it needs is on its way: the runtime's download and its install,
//! the start after them, or why there is no engine and how to get one. It is drawn in the terminal's colors like the
//! failure over a page that did not load (`panel::failure`), and the strip above it keeps working. It has no upstream
//! counterpart: macOS always has its web engine.

use std::time::{Duration, Instant};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, px, relative, svg,
};
use ngterm_update::web_engine::{MIN_VERSION, Progress};

use super::panel::PanelColors;
use crate::chrome::button::Button;

/// The bar's greatest width; a narrow pane shrinks it.
const BAR_WIDTH: f32 = 240.;
/// How often a running download redraws the pages.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Why there is no engine yet, and what the user can do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Unavailable {
    pub(super) title: &'static str,
    /// What went wrong, also the page's load error.
    pub(super) reason: String,
    /// The way that needs no download, when the reason does not say it already.
    pub(super) how_to: Option<String>,
}

impl Unavailable {
    /// A download that failed: its reason, and the installs that need no download.
    pub(super) fn download_failed(reason: String) -> Self {
        Self {
            title: "The web engine could not be downloaded",
            reason,
            how_to: Some(format!(
                "You can also install {}, or run ngterm --install-web-engine with the engine's archive, then try again.",
                system_package()
            )),
        }
    }

    /// A build that cannot download (it has no release pins, or `cause` says they are broken): only the system's
    /// package helps, since `--install-web-engine` checks an archive against the same pins.
    pub(super) fn not_installed(cause: Option<String>) -> Self {
        let install = format!("Install {}, then try again.", system_package());
        let (reason, how_to) = match cause {
            Some(cause) => (cause, Some(install)),
            None => (install, None),
        };
        Self {
            title: "The web engine is not installed",
            reason,
            how_to,
        }
    }
}

fn system_package() -> String {
    format!("WPE WebKit {MIN_VERSION} or newer from your distribution's packages")
}

/// What a waiting page shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Wait {
    Downloading {
        received: u64,
        total: u64,
    },
    /// The worker is starting after a download or a retry; the app's ordinary start shows nothing.
    Starting,
    Unavailable(Unavailable),
}

/// Which of a running download's reports reach the main thread: the first, the last, and one per interval.
#[derive(Debug, Default)]
pub(super) struct Throttle {
    last: Option<Instant>,
}

impl Throttle {
    pub(super) fn due(&mut self, progress: Progress, now: Instant) -> bool {
        let due = progress.received >= progress.total
            || self
                .last
                .is_none_or(|last| now.saturating_duration_since(last) >= PROGRESS_INTERVAL);
        if due {
            self.last = Some(now);
        }
        due
    }
}

/// `received` of `total` as the page reads it, in megabytes.
fn amount(received: u64, total: u64) -> String {
    let megabytes = |bytes: u64| bytes.saturating_add(500_000) / 1_000_000;
    format!("{} of {} MB", megabytes(received), megabytes(total))
}

/// How much of the bar is filled, within 0...1.
#[expect(
    clippy::cast_precision_loss,
    reason = "a fraction for a bar some hundred pixels wide"
)]
fn fraction(received: u64, total: u64) -> f32 {
    if total == 0 {
        return 0.0;
    }
    (received as f32 / total as f32).clamp(0.0, 1.0)
}

/// The panel over a page waiting in `wait`; `error` is the unavailable reason as selectable text, when the page has
/// it.
pub(super) fn element(wait: &Wait, colors: &PanelColors, error: Option<AnyElement>) -> AnyElement {
    let panel = div()
        .id("htmlOverlay.engine")
        .debug_selector(|| "htmlOverlay.engine".to_owned())
        .accessibility_id("htmlOverlay.engine")
        .role(match wait {
            Wait::Unavailable(_) => Role::Alert,
            _ => Role::Status,
        })
        // the page under it takes no press
        .occlude()
        .absolute()
        .inset_0()
        .bg(colors.background())
        .text_color(colors.ink(1.0))
        .p(px(24.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.));
    match wait {
        Wait::Downloading { received, total } if received >= total => {
            progress(panel, "Installing the web engine", 1.0, None, colors)
        }
        Wait::Downloading { received, total } => progress(
            panel,
            "Downloading the web engine",
            fraction(*received, *total),
            Some(amount(*received, *total)),
            colors,
        ),
        Wait::Starting => progress(panel, "Starting the web engine", 1.0, None, colors),
        Wait::Unavailable(unavailable) => unavailable_body(panel, unavailable, colors, error),
    }
    .into_any_element()
}

fn heading(icon: &'static str, title: &'static str, colors: &PanelColors) -> [AnyElement; 2] {
    [
        svg()
            .path(icon)
            .size(px(24.))
            .text_color(colors.ink(1.0))
            .into_any_element(),
        div().font_weight(FontWeight::BOLD).child(title).into_any_element(),
    ]
}

fn progress(
    panel: gpui_kit::Stateful<gpui_kit::Div>,
    title: &'static str,
    filled: f32,
    amount: Option<String>,
    colors: &PanelColors,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let label = SharedString::from(amount.clone().unwrap_or_else(|| title.to_owned()));
    panel
        .children(heading("icons/download.svg", title, colors))
        .child(
            div()
                .id("htmlOverlay.engine-progress")
                .debug_selector(|| "htmlOverlay.engine-progress".to_owned())
                .accessibility_id("htmlOverlay.engine-progress")
                .role(Role::ProgressIndicator)
                .aria_value(label)
                .mt(px(4.))
                .w(px(BAR_WIDTH))
                .max_w_full()
                .h(px(4.))
                .rounded(px(2.))
                .overflow_hidden()
                .bg(colors.ink(0.15))
                .child(div().h_full().w(relative(filled)).rounded(px(2.)).bg(colors.ink(1.0))),
        )
        .when_some(amount, |panel, amount| {
            panel.child(
                div()
                    .text_size(px(12.))
                    .text_color(colors.ink(0.7))
                    .child(SharedString::from(amount)),
            )
        })
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors.ink(0.55))
                .child("This page opens when it is ready."),
        )
}

fn unavailable_body(
    panel: gpui_kit::Stateful<gpui_kit::Div>,
    unavailable: &Unavailable,
    colors: &PanelColors,
    error: Option<AnyElement>,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    // the reason as the element made it (selectable text), else as plain text
    let reason = error.unwrap_or_else(|| SharedString::from(unavailable.reason.clone()).into_any_element());
    let ink = colors.ink(1.0);
    panel
        .children(heading("icons/triangle-alert.svg", unavailable.title, colors))
        .child(
            div()
                .debug_selector(|| "htmlOverlay.engine-reason".to_owned())
                // the reason's own box is as wide as its text and wraps at this width, centered in it
                .w_full()
                .flex()
                .justify_center()
                .text_size(px(12.))
                .text_center()
                .child(reason),
        )
        .when_some(unavailable.how_to.clone(), |panel, how_to| {
            panel.child(
                div()
                    .debug_selector(|| "htmlOverlay.engine-how-to".to_owned())
                    .max_w(px(480.))
                    .text_size(px(12.))
                    .text_center()
                    .text_color(colors.ink(0.7))
                    .child(SharedString::from(how_to)),
            )
        })
        .child(
            Button::new("htmlOverlay.engine-retry")
                .mt(px(4.))
                .h(px(26.))
                .px(px(12.))
                .rounded(px(5.))
                .border_1()
                .border_color(colors.ink(0.3))
                .text_size(px(12.))
                .wash(ink)
                .child("Try again")
                .on_click(|_, _, cx| {
                    cx.stop_propagation();
                    super::registry::retry_engine(cx);
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(received: u64) -> Progress {
        Progress { received, total: 100 }
    }

    #[test]
    fn a_running_download_reports_first_and_last_and_once_per_interval() {
        let mut throttle = Throttle::default();
        let start = Instant::now();
        assert!(throttle.due(at(0), start), "the first report");
        assert!(!throttle.due(at(1), start + Duration::from_millis(10)));
        assert!(!throttle.due(at(2), start + Duration::from_millis(99)));
        assert!(throttle.due(at(3), start + PROGRESS_INTERVAL), "an interval passed");
        assert!(!throttle.due(at(4), start + PROGRESS_INTERVAL));
        assert!(throttle.due(at(100), start + PROGRESS_INTERVAL), "the last report");
    }

    #[test]
    fn amounts_read_in_rounded_megabytes_and_the_bar_stays_within_its_track() {
        assert_eq!(amount(0, 112_000_000), "0 of 112 MB");
        assert_eq!(amount(41_499_999, 112_400_000), "41 of 112 MB");
        assert_eq!(amount(41_500_000, 112_500_000), "42 of 113 MB");
        assert!((fraction(1, 4) - 0.25).abs() < f32::EPSILON);
        assert!((fraction(9, 4) - 1.0).abs() < f32::EPSILON);
        assert!(fraction(1, 0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_waiting_pages_icons_are_served() {
        for path in ["icons/download.svg", "icons/triangle-alert.svg"] {
            assert!(crate::titlebar::serves(path), "{path} is not among the app's assets");
        }
    }

    #[test]
    fn a_failed_download_offers_the_ways_that_need_none() {
        let failed = Unavailable::download_failed("connection refused".to_owned());
        assert_eq!(failed.reason, "connection refused");
        let how_to = failed.how_to.unwrap();
        assert!(
            how_to.contains(&format!("WPE WebKit {MIN_VERSION} or newer")),
            "{how_to}"
        );
        assert!(how_to.contains("ngterm --install-web-engine"), "{how_to}");
    }

    #[test]
    fn a_build_without_pins_names_only_the_system_package() {
        let plain = Unavailable::not_installed(None);
        assert!(plain.reason.starts_with("Install WPE WebKit"), "{}", plain.reason);
        assert!(!plain.reason.contains("--install-web-engine"));
        assert_eq!(plain.how_to, None);
        let broken = Unavailable::not_installed(Some("Invalid web engine integrity pins.".to_owned()));
        assert_eq!(broken.reason, "Invalid web engine integrity pins.");
        assert!(broken.how_to.unwrap().starts_with("Install WPE WebKit"));
    }
}
