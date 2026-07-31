// SPDX-License-Identifier: MPL-2.0

//! Tracking `org.freedesktop.ScreenSaver` inhibits.
//!
//! # Why this is not just an idle inhibitor
//!
//! There are two unrelated ways an app says "keep the screen awake", and they
//! reach different places:
//!
//! - `zwp_idle_inhibit_manager_v1`, a Wayland protocol. The compositor honours
//!   it by never reporting the seat idle, so it suppresses our screensaver for
//!   free. mpv and VLC take this route. [`crate::idle_inhibit`] can observe it.
//! - `org.freedesktop.ScreenSaver.Inhibit`, a D-Bus call. On COSMIC that name is
//!   owned by cosmic-idle, which disables *its own* screen-off and lock timers
//!   and tells the compositor nothing. `ext-idle-notify-v1` keeps counting, so
//!   the seat still goes idle and our screensaver would come up over the video.
//!   Chrome takes this route:
//!
//!   ```text
//!   method call → org.freedesktop.ScreenSaver.Inhibit
//!     "/usr/bin/google-chrome-stable"
//!     "Video Wake Lock"
//!   ```
//!
//! This module covers the second case, so a video in Chrome is not interrupted
//! by a screensaver.
//!
//! # Why eavesdropping
//!
//! Only the owner of a D-Bus name sees calls made to it, and cosmic-idle owns
//! `org.freedesktop.ScreenSaver`. Taking the name away from it would cost the
//! user the screen-off and the lock, which matter far more than the screensaver.
//! Nor does cosmic-idle publish its inhibit state anywhere.
//!
//! So this becomes a passive bus monitor — `BecomeMonitor`, the same mechanism
//! `dbus-monitor` uses — and watches the calls go past. The bus grants that to
//! the owner of a session bus, and a failure to become one is not fatal: the
//! daemon just goes back to not knowing.
//!
//! # Bookkeeping
//!
//! `Inhibit` returns a cookie that `UnInhibit` passes back, but the cookie is
//! chosen by cosmic-idle and travels in a *reply*, which would mean monitoring
//! every method return on the bus to correlate. Counting per sender instead
//! gives the same answer for what we need — is anyone holding one — with a
//! fraction of the traffic. Clients that exit without releasing are cleaned up
//! from `NameOwnerChanged`.

use std::collections::HashMap;

use futures::StreamExt;
use tokio::sync::watch;
use zbus::{Connection, MessageStream, message::Type as MessageType};

pub use calloop::channel::Sender as CalloopSender;

/// The bus name whose inhibits we watch, owned by cosmic-idle on COSMIC.
const SCREENSAVER_NAME: &str = "org.freedesktop.ScreenSaver";

/// Longest app name we pass on for display, so an odd or hostile name cannot
/// run away with the settings window's layout.
const MAX_APP_NAME_LEN: usize = 64;

/// An app currently asking that the screen stay awake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inhibitor {
    /// The application name the app passed to `Inhibit`, reduced to its file
    /// name and truncated. Empty when the app gave nothing usable.
    pub app: String,
    /// The reason it gave, e.g. `"Video Wake Lock"`.
    pub reason: String,
}

impl Inhibitor {
    /// The app name to show a user, or `None` when it gave nothing usable.
    #[must_use]
    pub fn display_name(&self) -> Option<&str> {
        (!self.app.is_empty()).then_some(self.app.as_str())
    }
}

/// Sent over the calloop channel when the set of inhibitors changes.
#[derive(Debug, Clone, Copy)]
pub struct InhibitorsChanged;

/// Read side of the monitor: the inhibitors held right now.
#[derive(Clone)]
pub struct InhibitMonitorHandle {
    rx: watch::Receiver<Vec<Inhibitor>>,
}

impl InhibitMonitorHandle {
    /// The inhibitors held right now.
    #[must_use]
    pub fn current(&self) -> Vec<Inhibitor> {
        self.rx.borrow().clone()
    }

    /// Whether anything is asking for the screen to stay awake.
    #[must_use]
    pub fn is_inhibited(&self) -> bool {
        !self.rx.borrow().is_empty()
    }

    /// Wait for the set of inhibitors to change, returning the new set.
    ///
    /// Returns `None` once the monitor has stopped, so a caller driving this in
    /// a loop ends with it.
    pub async fn changed(&mut self) -> Option<Vec<Inhibitor>> {
        self.rx.changed().await.ok()?;
        Some(self.rx.borrow_and_update().clone())
    }
}

/// One client's inhibits, keyed by its unique bus name.
///
/// A client may hold several at once (a browser with two playing tabs), so this
/// counts rather than storing a flag.
#[derive(Debug)]
struct Held {
    app: String,
    reason: String,
    count: u32,
}

/// Start watching `org.freedesktop.ScreenSaver` inhibits in the background.
///
/// Returns `None` if the monitor could not be started at all, in which case
/// inhibits simply go unnoticed. `notify_tx`, when given, receives a message on
/// every change so a calloop loop can wake up and react.
#[must_use]
pub fn start_screensaver_inhibit_monitor(
    notify_tx: Option<CalloopSender<InhibitorsChanged>>,
) -> Option<InhibitMonitorHandle> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .inspect_err(|err| tracing::warn!(?err, "Could not build inhibit monitor runtime"))
        .ok()?;

    let (tx, rx) = watch::channel(Vec::new());

    std::thread::Builder::new()
        .name("screensaver-inhibit".into())
        .spawn(move || {
            runtime.block_on(async move {
                match monitor_loop(&tx, notify_tx.as_ref()).await {
                    Ok(()) => tracing::info!("Screensaver inhibit monitor ended"),
                    Err(err) => tracing::warn!(
                        ?err,
                        "Screensaver inhibit monitor stopped; D-Bus screen-wake requests \
                         will not be noticed"
                    ),
                }
            });
        })
        .inspect_err(|err| tracing::warn!(?err, "Could not spawn the inhibit monitor thread"))
        .ok()?;

    Some(InhibitMonitorHandle { rx })
}

/// Watch the bus until the connection ends.
async fn monitor_loop(
    tx: &watch::Sender<Vec<Inhibitor>>,
    notify_tx: Option<&CalloopSender<InhibitorsChanged>>,
) -> zbus::Result<()> {
    let connection = Connection::session().await?;

    // Only the two calls we care about, plus name changes so a client that dies
    // mid-playback does not hold the screensaver off forever. Narrow rules keep
    // this from becoming a firehose: a monitor sees every message it matches.
    let rules = [
        format!("type='method_call',interface='{SCREENSAVER_NAME}',member='Inhibit'"),
        format!("type='method_call',interface='{SCREENSAVER_NAME}',member='UnInhibit'"),
        "type='signal',interface='org.freedesktop.DBus',member='NameOwnerChanged'".to_owned(),
    ];
    let rules: Vec<&str> = rules.iter().map(String::as_str).collect();

    // After this the connection can only receive, which is why the call comes
    // before the stream and nothing else is sent on it.
    connection
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus.Monitoring"),
            "BecomeMonitor",
            &(rules.as_slice(), 0u32),
        )
        .await?;

    tracing::info!("Watching for D-Bus screen-wake requests");

    let mut held: HashMap<String, Held> = HashMap::new();
    let mut stream = MessageStream::from(&connection);

    loop {
        let message = tokio::select! {
            message = stream.next() => match message {
                Some(message) => message,
                None => break,
            },
            // Every handle is gone, so nobody is listening any more. Matters for
            // the settings app, which starts a monitor per visit to the
            // screensaver settings and would otherwise leave one behind each time.
            () = tx.closed() => {
                tracing::debug!("No listeners left; stopping the inhibit monitor");
                break;
            }
        };

        let Ok(message) = message else {
            // A malformed message says nothing about the ones that follow.
            continue;
        };

        let header = message.header();
        let Some(member) = header.member().map(zbus::names::MemberName::as_str) else {
            continue;
        };
        let sender = header
            .sender()
            .map(std::string::ToString::to_string)
            .unwrap_or_default();

        match (message.message_type(), member) {
            (MessageType::MethodCall, "Inhibit") => {
                // Signature is (app_name, reason); a caller that sends something
                // else is not inhibiting anything either.
                let Ok((app, reason)) = message.body().deserialize::<(String, String)>() else {
                    continue;
                };
                let entry = held.entry(sender.clone()).or_insert_with(|| Held {
                    app: String::new(),
                    reason: String::new(),
                    count: 0,
                });
                entry.app = tidy_app_name(&app);
                entry.reason = reason;
                entry.count = entry.count.saturating_add(1);
                tracing::debug!(
                    sender = %sender,
                    app = %entry.app,
                    reason = %entry.reason,
                    count = entry.count,
                    "App asked to keep the screen awake"
                );
            }

            (MessageType::MethodCall, "UnInhibit") => {
                // The cookie is not checked: per-sender counting is what we
                // track, so releasing one of a client's inhibits is enough.
                if let Some(entry) = held.get_mut(&sender) {
                    entry.count = entry.count.saturating_sub(1);
                    if entry.count == 0 {
                        held.remove(&sender);
                    }
                    tracing::debug!(sender = %sender, "App released a screen-wake request");
                }
            }

            (MessageType::Signal, "NameOwnerChanged") => {
                let Ok((name, _old, new_owner)) =
                    message.body().deserialize::<(String, String, String)>()
                else {
                    continue;
                };
                // An empty new owner means the name went away; for a unique name
                // that is the client disconnecting, inhibits and all.
                if new_owner.is_empty() && held.remove(&name).is_some() {
                    tracing::debug!(name = %name, "Inhibiting app disconnected");
                }
            }

            _ => continue,
        }

        publish(&held, tx, notify_tx);
    }

    Ok(())
}

/// Publish the current set, if it differs from what was published last.
fn publish(
    held: &HashMap<String, Held>,
    tx: &watch::Sender<Vec<Inhibitor>>,
    notify_tx: Option<&CalloopSender<InhibitorsChanged>>,
) {
    let mut inhibitors: Vec<Inhibitor> = held
        .values()
        .map(|entry| Inhibitor {
            app: entry.app.clone(),
            reason: entry.reason.clone(),
        })
        .collect();
    // HashMap order is arbitrary; sort so an unchanged set compares equal.
    inhibitors.sort_by(|a, b| (&a.app, &a.reason).cmp(&(&b.app, &b.reason)));

    if *tx.borrow() == inhibitors {
        return;
    }

    tracing::info!(
        count = inhibitors.len(),
        ?inhibitors,
        "Screen-wake requests changed"
    );
    let _ = tx.send(inhibitors);

    if let Some(notify_tx) = notify_tx {
        let _ = notify_tx.send(InhibitorsChanged);
    }
}

/// Reduce the name an app passes to `Inhibit` to something showable.
///
/// Apps tend to pass their executable path (`/usr/bin/google-chrome-stable`),
/// so this keeps the file name. Anything unusable becomes empty, which callers
/// read as "an app" rather than printing a path fragment.
fn tidy_app_name(raw: &str) -> String {
    let name = raw
        .trim()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_APP_NAME_LEN)
        .collect::<String>();

    name.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_paths_reduce_to_their_file_name() {
        assert_eq!(
            tidy_app_name("/usr/bin/google-chrome-stable"),
            "google-chrome-stable"
        );
        assert_eq!(tidy_app_name("firefox"), "firefox");
    }

    #[test]
    fn unusable_names_become_empty() {
        assert_eq!(tidy_app_name(""), "");
        assert_eq!(tidy_app_name("   "), "");
        assert_eq!(tidy_app_name("/usr/bin/"), "");
    }

    #[test]
    fn long_names_are_truncated() {
        let long = "x".repeat(MAX_APP_NAME_LEN * 2);
        assert_eq!(tidy_app_name(&long).chars().count(), MAX_APP_NAME_LEN);
    }

    #[test]
    fn control_characters_are_dropped() {
        assert_eq!(tidy_app_name("ev\u{7}il\n"), "evil");
    }

    #[test]
    fn nameless_inhibitor_has_no_display_name() {
        let inhibitor = Inhibitor {
            app: String::new(),
            reason: "Video Wake Lock".to_owned(),
        };
        assert_eq!(inhibitor.display_name(), None);
    }

    #[test]
    fn named_inhibitor_shows_its_name() {
        let inhibitor = Inhibitor {
            app: "mpv".to_owned(),
            reason: String::new(),
        };
        assert_eq!(inhibitor.display_name(), Some("mpv"));
    }
}
