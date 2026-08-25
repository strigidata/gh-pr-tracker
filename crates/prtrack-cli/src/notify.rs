//! Posting a desktop notification, and the macOS workaround that makes it
//! arrive.
//!
//! `prtrack` only earns its keep if an interruption actually reaches the
//! developer. This began as the delivery spike that proved the path works and
//! is kept here, unchanged in substance, because the watcher will need it and
//! because [`adopt_bundle_identity`] encodes a finding that cost a day to get
//! and is invisible from the API it works around.
//!
//! Every line it prints is diagnostics, which is why none of it goes to
//! stdout.

use anyhow::{Context as _, Result};
use notify_rust::Notification;

/// Whether a Focus mode is swallowing notifications.
///
/// The whole point of this tool is that an interruption arrives, so "it was
/// posted and you will never see it" is the one outcome that must not pass
/// unremarked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Focus {
    /// A mode is on. Anything posted now is dropped without a sound, and
    /// nothing in the notification API says so.
    Active {
        /// The mode's identifier, such as `com.apple.focus.work`.
        mode: String,
    },
    /// No mode is on.
    Clear,
    /// Cannot be told from here: not macOS, or the file is missing, or it is a
    /// shape this does not recognise.
    ///
    /// Deliberately not folded into [`Focus::Clear`]. "No Focus is on" and "I
    /// could not find out" lead to different things being said to the user, and
    /// collapsing them would make the quieter, wronger one the default.
    Unknown,
}

/// Where macOS records the Focus modes currently asserted.
///
/// Private, undocumented, and not a stable interface — which is why every
/// failure to read or parse it becomes [`Focus::Unknown`] rather than an error.
/// Being unable to check must never stop a nudge being posted.
#[cfg(target_os = "macos")]
const FOCUS_ASSERTIONS: &str = "Library/DoNotDisturb/DB/Assertions.json";

/// Reads whether a Focus mode is currently on.
///
/// Never fails: an unreadable or unfamiliar file is [`Focus::Unknown`]. This is
/// a courtesy check on a private file, and a tool that refused to run because
/// Apple moved a JSON file would be worse than one that shrugs.
#[must_use]
pub fn focus() -> Focus {
    #[cfg(not(target_os = "macos"))]
    {
        Focus::Unknown
    }

    #[cfg(target_os = "macos")]
    {
        let Some(home) = std::env::var_os("HOME") else {
            return Focus::Unknown;
        };
        let path = std::path::Path::new(&home).join(FOCUS_ASSERTIONS);
        let Ok(raw) = std::fs::read_to_string(path) else {
            return Focus::Unknown;
        };
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&raw) else {
            return Focus::Unknown;
        };
        read_focus(&parsed)
    }
}

/// Picks the active mode out of the assertions document.
///
/// Split from [`focus`] so the shape can be tested without a real file, which
/// is the only part of this worth testing: reading a path is not where the
/// mistakes are.
fn read_focus(parsed: &serde_json::Value) -> Focus {
    let Some(entries) = parsed.get("data").and_then(serde_json::Value::as_array) else {
        return Focus::Unknown;
    };

    // `storeAssertionRecords` holds what is asserted *now*;
    // `storeInvalidationRecords` beside it holds modes that have ended, and
    // reading that one instead would report a Focus that finished hours ago.
    let mut saw_the_field = false;
    for entry in entries {
        let Some(active) = entry
            .get("storeAssertionRecords")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        saw_the_field = true;
        if let Some(mode) = active.iter().find_map(|assertion| {
            assertion
                .get("assertionDetails")
                .and_then(|details| details.get("assertionDetailsModeIdentifier"))
                .and_then(serde_json::Value::as_str)
        }) {
            return Focus::Active {
                mode: mode.to_owned(),
            };
        }
    }

    if saw_the_field {
        Focus::Clear
    } else {
        Focus::Unknown
    }
}

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

#[cfg(test)]
mod tests {
    use super::{read_focus, Focus};
    use serde_json::json;

    #[test]
    fn an_active_mode_is_read_and_an_ended_one_is_not() {
        // The two arrays sit side by side and hold the same shape. Reading the
        // wrong one reports a Focus that ended hours ago, which is worse than
        // reporting none: it tells somebody their notifications are suppressed
        // when they are not, and the advice that follows is wrong.
        let active = json!({"data": [{
            "storeAssertionRecords": [{
                "assertionDetails": {"assertionDetailsModeIdentifier": "com.apple.focus.work"}
            }],
            "storeInvalidationRecords": [{
                "invalidationAssertion": {
                    "assertionDetails": {"assertionDetailsModeIdentifier": "com.apple.focus.sleep"}
                }
            }]
        }]});
        assert_eq!(
            read_focus(&active),
            Focus::Active {
                mode: "com.apple.focus.work".to_owned()
            },
            "the asserted mode, not the invalidated one"
        );

        // A finished Focus with nothing currently asserted is Clear. This is
        // the state the machine is in most of the time, and calling it Active
        // would make the warning permanent and therefore ignored.
        let ended = json!({"data": [{
            "storeAssertionRecords": [],
            "storeInvalidationRecords": [{
                "invalidationAssertion": {
                    "assertionDetails": {"assertionDetailsModeIdentifier": "com.apple.focus.work"}
                }
            }]
        }]});
        assert_eq!(read_focus(&ended), Focus::Clear, "nothing asserted now");
    }

    #[test]
    fn a_shape_this_does_not_know_is_unknown_rather_than_clear() {
        // Apple owns this file and may change it. Saying "no Focus is on" when
        // the truth is "I could not tell" is the failure that matters: it is
        // the quiet answer, so it is the one that would go unnoticed.
        for unfamiliar in [
            json!({}),
            json!({"data": "not an array"}),
            json!({"data": [{"somethingElse": []}]}),
        ] {
            assert_eq!(read_focus(&unfamiliar), Focus::Unknown, "{unfamiliar}");
        }
    }
}
