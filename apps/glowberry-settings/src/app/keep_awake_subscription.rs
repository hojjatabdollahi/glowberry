// SPDX-License-Identifier: MPL-2.0

//! Subscription reporting whether a Wayland idle inhibitor is held.
//!
//! Apps keep the screen awake in two unrelated ways. A Wayland idle inhibitor
//! (mpv, VLC) can only be *inferred*, by comparing two idle notifications — see
//! [`glowberry_lib::idle_inhibit`] — so it needs a live watcher, which is what
//! this wraps. The other way, an `org.freedesktop.ScreenSaver` D-Bus call
//! (Chrome), needs no subscription: such requests are only observable as they
//! are made, so the daemon watches for them from login and publishes them to
//! state, and the settings app reads them from there.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cosmic::iced::Subscription;
use cosmic::iced::futures::{Stream, StreamExt as _, channel::mpsc};

/// Watch for idle inhibitors for as long as this subscription is alive.
///
/// Emits `true` when something is holding an idle inhibitor and `false` when
/// nothing is, and stays quiet while the user is active — inhibition is only
/// observable once input has been quiet for a couple of seconds.
pub fn idle_inhibited() -> Subscription<bool> {
    Subscription::run(watch)
}

/// Sets the watcher's stop flag when the stream is dropped, so the thread and
/// its Wayland connection go away with the subscription.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

fn watch() -> impl Stream<Item = bool> {
    // The watcher blocks on its own Wayland event loop, so it gets a plain
    // thread rather than a place in the async runtime.
    let (mut sender, receiver) = mpsc::channel(1);
    let stop = Arc::new(AtomicBool::new(false));
    let guard = StopOnDrop(Arc::clone(&stop));

    std::thread::Builder::new()
        .name("idle-inhibit-probe".into())
        .spawn(move || {
            let result = glowberry_lib::idle_inhibit::watch(&stop, |inhibited| {
                // A full channel means an unread verdict is already queued and
                // this one supersedes it; the next change will catch up.
                let _ = sender.try_send(inhibited);
            });

            match result {
                Ok(()) => tracing::debug!("Idle inhibitor probe stopped"),
                // An unsupported compositor is expected on anything but
                // cosmic-comp, and simply means no warning is ever shown.
                Err(glowberry_lib::idle_inhibit::ProbeError::Unsupported) => {
                    tracing::info!(
                        "Compositor cannot report idle inhibitors; skipping the warning"
                    );
                }
                Err(err) => tracing::warn!(?err, "Idle inhibitor probe failed"),
            }
        })
        .map_err(|err| tracing::warn!(?err, "Could not spawn the idle inhibitor probe"))
        .ok();

    // `guard` rides along in the stream so it is dropped with it.
    receiver.map(move |inhibited| {
        let _ = &guard;
        inhibited
    })
}
