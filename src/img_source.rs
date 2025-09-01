// SPDX-License-Identifier: MPL-2.0

use notify::event::{ModifyKind, RenameMode};
use sctk::reexports::calloop::{LoopHandle, channel};

use crate::CosmicBg;

pub fn img_source(handle: &LoopHandle<CosmicBg>) -> channel::SyncSender<(String, notify::Event)> {
    let (notify_tx, notify_rx) = channel::sync_channel(20);
    let _res = handle
        .insert_source(
            notify_rx,
            |e: channel::Event<(String, notify::Event)>, _, state| {
                match e {
                    channel::Event::Msg((source, event)) => match event.kind {
                        notify::EventKind::Create(_)
                        | notify::EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                            for w in state
                                .wallpapers
                                .iter_mut()
                                .filter(|w| w.entry.output == source)
                            {
                                for p in &event.paths {
                                    if !w.image_queue.contains(p) {
                                        w.image_queue.push_front(p.into());
                                    }
                                }
                                w.image_queue.retain(|p| !event.paths.contains(p));
                                // TODO maybe resort or shuffle at some point?
                            }
                        }
                        notify::EventKind::Remove(_)
                        | notify::EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                            for w in state
                                .wallpapers
                                .iter_mut()
                                .filter(|w| w.entry.output == source)
                            {
                                w.image_queue.retain(|p| !event.paths.contains(p));
                            }
                        }
                        _ => {}
                    },
                    channel::Event::Closed => {
                        // TODO log drop
                    }
                }
            },
        )
        .map(|_| {})
        .map_err(|err| eyre::eyre!("{}", err));

    notify_tx
}
