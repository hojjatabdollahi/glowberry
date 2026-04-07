// SPDX-License-Identifier: MPL-2.0

use notify::Event;
use sctk::reexports::calloop::{LoopHandle, channel};

pub fn img_source<T, F>(
    handle: &LoopHandle<T>,
    mut on_event: F,
) -> channel::SyncSender<(String, Event)>
where
    F: FnMut(&mut T, String, Event) + 'static,
{
    let (notify_tx, notify_rx) = channel::sync_channel(20);
    let _res = handle
        .insert_source(
            notify_rx,
            move |e: channel::Event<(String, Event)>, _, state| match e {
                channel::Event::Msg((source, event)) => on_event(state, source, event),
                channel::Event::Closed => {}
            },
        )
        .map(|_| {})
        .map_err(|err| eyre::eyre!("{}", err));

    notify_tx
}
