//! Posting a desktop notification, and the macOS workaround that makes it
//! arrive.
//!
//! `prtrack` only earns its keep if an interruption actually reaches the
//! developer. This began as the delivery spike that proved the path works and
//! is kept here, unchanged in substance, because the watcher will need it and
//! because [`adopt_bundle_identity`] encodes a finding that cost a day to get
//! and is invisible from the API it works around.
//!
//! **Nothing calls this yet.** It is not dead code in the sense the lint
//! means — it is the shell half of a feature whose other half is not written —
//! but it is also not exercised, so treat its behaviour as last verified by
//! the spike rather than by the suite.
//!
//! Every line it prints is diagnostics, which is why none of it goes to
//! stdout.

use anyhow::{Context as _, Result};
use notify_rust::Notification;

/// Posts one notification.
///
/// **Whether it is displayed is not knowable from here.** The backend accepts
/// the request and returns success; macOS then silently drops it if
/// notifications are off for the owning application, or a Focus mode is
/// filtering them. There is no callback and no query, so a suppressed nudge
/// and a delivered one are indistinguishable to this process. A tool whose
/// whole thesis is the interruption must not claim otherwise, which is why
/// nothing above this reports a nudge as "delivered".
///
/// # Errors
///
/// Returns whatever the notification backend rejected the request with.
pub fn post(summary: &str, body: &str) -> Result<()> {
    // The bundle identity is *not* adopted here. `set_application` is guarded
    // by a `Once`, so a second call reports `AlreadySet` — which this function
    // would report as a failure, turning every notification after the first
    // into an error. The caller adopts once, before the first post.
    //
    // The Result is propagated rather than dropped on purpose. A discarded
    // error here is exactly how a broken notification path stays invisible.
    Notification::new()
        .summary(summary)
        .body(body)
        .show()
        .context("the notification backend rejected the request")?;

    Ok(())
}

/// Borrows a bundle identifier so macOS will accept our notification, and
/// returns the identifier that was adopted.
///
/// # Errors
///
/// Returns the failure from `set_application` if the identifier is refused.
///
/// macOS attributes every notification to an application bundle. A bare
/// `cargo`-built binary has no bundle, and the failure mode is silence rather
/// than an error: the call returns success and nothing appears on screen. See
/// <https://github.com/hoodie/notify-rust/issues/132>. The workaround is to
/// borrow the identifier of an application that does have a bundle, so the
/// notification is attributed to it — which means the banner carries that
/// application's name and icon, not ours. A shipped build must replace all of
/// this with a real signed `.app` bundle.
///
/// Which identifier to borrow is not a free choice, and the obvious candidate
/// is the wrong one. Borrowing the host terminal (`__CFBundleIdentifier`, set
/// by Launch Services and inherited down the process tree) fails on any
/// terminal that already talks to `usernoted` over the modern
/// `UNUserNotificationCenter` protocol: `notify-rust`'s default backend is the
/// legacy `NSUserNotification` one, and `usernoted` refuses to serve both
/// under one identifier. Measured on macOS 27.0 against `dev.warp.Warp-Stable`,
/// it logs
///
/// ```text
/// Legacy client dev.warp.Warp-Stable connecting to modern client.
///   You can't mix modern clients with legacy clients.
/// Denying message 3 from connection <LegacyConnection ...>
/// ```
///
/// three runs out of three, drops the notification, and still returns success
/// to us. So we borrow an application that stays on the legacy path instead.
/// `get_bundle_identifier_or_default` falls back to `com.apple.Finder` when
/// even that lookup fails, which is why this cannot return "no identity".
///
/// The override exists so the choice can be re-probed without a rebuild; it is
/// a spike knob, not a product setting.
///
/// This must run before the first `show()`. `set_application` is guarded by a
/// `Once`, and `mac-notification-sys` calls it with a default of its own on
/// the first send, so a later call would only report `AlreadySet`.
#[cfg(target_os = "macos")]
pub fn adopt_bundle_identity() -> Result<String> {
    /// Application whose identifier we borrow. Chosen because it does not
    /// register as a modern notification client.
    const BORROWED_APP: &str = "Terminal";
    /// Escape hatch for probing a different identifier.
    const BUNDLE_OVERRIDE_ENV: &str = "PRTRACK_BUNDLE_ID";

    let bundle_id = std::env::var(BUNDLE_OVERRIDE_ENV)
        .unwrap_or_else(|_| notify_rust::get_bundle_identifier_or_default(BORROWED_APP));
    notify_rust::set_application(&bundle_id)
        .with_context(|| format!("could not adopt the bundle identifier {bundle_id}"))?;
    Ok(bundle_id)
}
