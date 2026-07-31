// SPDX-License-Identifier: MPL-2.0

//! Detecting whether something is holding an idle inhibitor.
//!
//! An application that holds a `zwp_idle_inhibitor_v1` on a visible surface —
//! a browser playing video, a presentation tool, a game — stops the compositor
//! from ever reporting the seat idle. GlowBerry's screensaver waits on
//! `ext-idle-notify-v1`, so while such an inhibitor is held the screensaver
//! silently never appears. That looks like a broken setting rather than an
//! intentional one, which is what this module exists to explain.
//!
//! Nothing in the protocol lets a client ask "is idle inhibited?", and the
//! inhibiting client is not identifiable from outside the compositor. But
//! version 2 of `ext-idle-notify-v1` added `get_input_idle_notification`, which
//! tracks user input alone and *ignores* inhibitors, while plain
//! `get_idle_notification` respects them. Arming both against the same seat and
//! comparing them answers the question: if input has gone quiet and the
//! inhibitor-respecting notification has not fired, something is inhibiting.
//!
//! The two timeouts are deliberately different, the respecting one being
//! shorter. By the time the input notification reports idle, an uninhibited
//! compositor has necessarily reported the other one already, so the difference
//! between the timeouts serves as a grace period without needing any timers.
//!
//! ```text
//! last input   0s        1.0s                    2.5s
//!              |----------|-----------------------|
//!                    respecting idles       input idles -> compare
//! ```
//!
//! Note this covers the compositor-level inhibit, which is the one that
//! actually suppresses the screensaver. An `org.freedesktop.ScreenSaver`
//! D-Bus inhibit goes to cosmic-idle instead and only disables *its* timeouts;
//! it does not stop us, and cosmic-idle exposes no way to enumerate it anyway.

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use sctk::reexports::{
    calloop::EventLoop,
    calloop_wayland_source::WaylandSource,
    client::{
        Connection, Dispatch, QueueHandle, delegate_noop,
        globals::{GlobalListContents, registry_queue_init},
        protocol::{wl_registry, wl_seat::WlSeat},
    },
    protocols::ext::idle_notify::v1::client::{ext_idle_notification_v1, ext_idle_notifier_v1},
};

/// Timeout for the notification that respects inhibitors.
const RESPECTING_TIMEOUT_MS: u32 = 1_000;

/// Timeout for the input-only notification. Later than
/// [`RESPECTING_TIMEOUT_MS`] so that an uninhibited compositor has always
/// reported the respecting notification first; the gap between the two is the
/// grace period.
const INPUT_TIMEOUT_MS: u32 = 2_500;

// The whole comparison rests on this ordering.
const _: () = assert!(RESPECTING_TIMEOUT_MS < INPUT_TIMEOUT_MS);

/// How long to block in the event loop before re-checking the stop flag.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Why the probe could not run.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// The compositor does not offer `ext-idle-notify-v1` version 2, so
    /// inhibitors cannot be observed. Nothing the user can act on.
    #[error("compositor does not support ext-idle-notify-v1 version 2")]
    Unsupported,
    /// The compositor has no seat, so idle cannot be tracked at all.
    #[error("no wl_seat available")]
    NoSeat,
    /// Anything else: no display to connect to, a broken registry, a failed
    /// event loop.
    #[error("{0}")]
    Failed(String),
}

/// Which of the two notifications an event belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Respects idle inhibitors, like the daemon's own notification.
    Respecting,
    /// Tracks input only, ignoring inhibitors.
    Input,
}

/// Idle state of both notifications.
#[derive(Debug, Default)]
struct Probe {
    respecting_idle: bool,
    input_idle: bool,
}

impl Probe {
    /// Whether an inhibitor is being held, or `None` while the user is active
    /// and nothing can be concluded yet.
    fn verdict(&self) -> Option<bool> {
        verdict_from(self.input_idle, self.respecting_idle)
    }
}

/// The inhibitor verdict for a pair of notification states.
///
/// Input going quiet is what makes a comparison possible: until then the
/// respecting notification is expected to be non-idle for the ordinary reason
/// that the user is using the machine.
fn verdict_from(input_idle: bool, respecting_idle: bool) -> Option<bool> {
    input_idle.then_some(!respecting_idle)
}

/// Watch for idle-inhibitor changes until `stop` is set, invoking `on_change`
/// with each new verdict.
///
/// Blocks on its own Wayland connection, so this is meant for a dedicated
/// thread. `on_change` fires only when the verdict actually changes, and never
/// while the user is active — a held inhibitor is only detectable once input
/// has been quiet for [`INPUT_TIMEOUT_MS`].
///
/// # Errors
///
/// Returns [`ProbeError::Unsupported`] on compositors without
/// `ext-idle-notify-v1` version 2, and [`ProbeError::Failed`] if the Wayland
/// connection or event loop cannot be set up.
pub fn watch(stop: &AtomicBool, mut on_change: impl FnMut(bool)) -> Result<(), ProbeError> {
    let conn = Connection::connect_to_env()
        .map_err(|err| ProbeError::Failed(format!("wayland connection failed: {err}")))?;

    let (globals, event_queue) = registry_queue_init::<Probe>(&conn)
        .map_err(|err| ProbeError::Failed(format!("wayland registry unavailable: {err}")))?;
    let qh = event_queue.handle();

    // Version 2 exactly: `get_input_idle_notification` is the whole point, and
    // without it there is nothing to compare against.
    let notifier = globals
        .bind::<ext_idle_notifier_v1::ExtIdleNotifierV1, _, _>(&qh, 2..=2, ())
        .map_err(|_| ProbeError::Unsupported)?;

    // Only the seat's identity matters here, so the oldest version will do.
    let seat = globals
        .bind::<WlSeat, _, _>(&qh, 1..=1, ())
        .map_err(|_| ProbeError::NoSeat)?;

    let respecting =
        notifier.get_idle_notification(RESPECTING_TIMEOUT_MS, &seat, &qh, Kind::Respecting);
    let input = notifier.get_input_idle_notification(INPUT_TIMEOUT_MS, &seat, &qh, Kind::Input);

    let mut event_loop: EventLoop<'static, Probe> = EventLoop::try_new()
        .map_err(|err| ProbeError::Failed(format!("failed to create event loop: {err}")))?;
    WaylandSource::new(conn, event_queue)
        .insert(event_loop.handle())
        .map_err(|err| ProbeError::Failed(format!("failed to insert wayland source: {err}")))?;

    tracing::debug!("Idle inhibitor probe armed");

    let mut probe = Probe::default();
    let mut reported: Option<bool> = None;

    while !stop.load(Ordering::Relaxed) {
        // The verdict is read after the whole dispatch rather than inside the
        // event handler, so a batch carrying both notifications' events is
        // judged once, on its final state.
        event_loop
            .dispatch(POLL_INTERVAL, &mut probe)
            .map_err(|err| ProbeError::Failed(format!("wayland dispatch failed: {err}")))?;

        if let Some(verdict) = probe.verdict()
            && reported != Some(verdict)
        {
            reported = Some(verdict);
            tracing::debug!(inhibited = verdict, "Idle inhibitor state changed");
            on_change(verdict);
        }
    }

    input.destroy();
    respecting.destroy();

    Ok(())
}

impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, Kind> for Probe {
    fn event(
        state: &mut Self,
        _notification: &ext_idle_notification_v1::ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        kind: &Kind,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let idle = match event {
            ext_idle_notification_v1::Event::Idled => true,
            ext_idle_notification_v1::Event::Resumed => false,
            _ => return,
        };

        match kind {
            Kind::Respecting => state.respecting_idle = idle,
            Kind::Input => state.input_idle = idle,
        }
    }
}

// The probe binds its globals up front and never reacts to later ones.
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Probe {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _contents: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(Probe: ext_idle_notifier_v1::ExtIdleNotifierV1);
delegate_noop!(Probe: ignore WlSeat);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_user_yields_no_verdict() {
        // Nothing can be concluded while input is still arriving.
        assert_eq!(verdict_from(false, false), None);
        assert_eq!(verdict_from(false, true), None);
    }

    #[test]
    fn quiet_input_with_idle_seat_is_not_inhibited() {
        assert_eq!(verdict_from(true, true), Some(false));
    }

    #[test]
    fn quiet_input_without_idle_seat_is_inhibited() {
        // Input has been quiet past the longer timeout, yet the compositor
        // still refuses to call the seat idle: something is inhibiting.
        assert_eq!(verdict_from(true, false), Some(true));
    }
}
