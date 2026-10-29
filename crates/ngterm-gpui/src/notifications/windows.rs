//! The banners on Windows: a seam that drains the queue and posts nothing yet. Windows toasts under ngterm's
//! AppUserModelID, their clicks and withdrawals arrive with Task 5 of `docs/plans/20261029-windows-support.md`
//! (Decisions > Notifications); until then the badge and the `notify` event work as everywhere, and each banner is
//! logged.

use super::{Banner, Click};

/// The banner task: takes `banners` in order and drops each with a log line (no toast host yet, see the module docs).
/// `app_id` and `clicked` are the toast host's. Ends when every sink is gone.
pub async fn run(banners: async_channel::Receiver<Banner>, app_id: String, clicked: impl Fn(Click) + Send + 'static) {
    let _ = clicked;
    tracing::info!(app_id, "notify: Windows toasts arrive in Task 5; banners are not shown");
    while let Ok(banner) = banners.recv().await {
        match banner {
            Banner::Send { identity, .. } => tracing::debug!(%identity, "notify: banner not shown (no toast host yet)"),
            Banner::Withdraw { identity } => {
                tracing::debug!(%identity, "notify: nothing to withdraw (no toast host yet)")
            }
        }
    }
}
