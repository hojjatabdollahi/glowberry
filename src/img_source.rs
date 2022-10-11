use std::{fs, path::PathBuf};

use notify::{
    event::{ModifyKind, RenameMode},
    RecommendedWatcher, RecursiveMode, Watcher,
};
use sctk::reexports::calloop::{
    channel::{self, channel},
    LoopHandle,
};

use crate::CosmicBg;

pub fn img_source(
    sources: Vec<(usize, PathBuf)>,
    handle: LoopHandle<CosmicBg>,
) -> (
    channel::Sender<(usize, notify::Event)>,
    Vec<RecommendedWatcher>,
) {
    let (notify_tx, notify_rx) = channel();
    let _ = handle
        .insert_source(
            notify_rx,
            |e: channel::Event<(usize, notify::Event)>, _, state| {
                match e {
                    channel::Event::Msg((source, event)) => match event.kind {
                        notify::EventKind::Create(_)
                        | notify::EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                            for w in state.wallpapers.iter_mut().filter(|w| w.id == source) {
                                for p in &event.paths {
                                    if !w.image_queue.contains(p) {
                                        w.image_queue.push_front(p.into());
                                    }
                                }
                                w.image_queue.retain(|p| !event.paths.contains(p));
                            }
                        }
                        notify::EventKind::Remove(_)
                        | notify::EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                            for w in state.wallpapers.iter_mut().filter(|w| w.id == source) {
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
        .map_err(|err| anyhow::anyhow!("{}", err));

    let notify_tx_clone = notify_tx.clone();
    (
        notify_tx,
        sources
            .into_iter()
            .filter_map(|(id, path_source)| {
                let tx_clone = notify_tx_clone.clone();
                let mut watcher = match RecommendedWatcher::new(
                    move |res| {
                        if let Ok(e) = res {
                            let _ = tx_clone.send((id, e));
                        }
                    },
                    notify::Config::default(),
                ) {
                    Ok(w) => w,
                    Err(_) => return None,
                };

                if let Ok(m) = fs::metadata(&path_source) {
                    if m.is_dir() {
                        let _ = watcher.watch(&path_source, RecursiveMode::Recursive);
                    } else if m.is_file() {
                        let _ = watcher.watch(&path_source, RecursiveMode::NonRecursive);
                    }
                }

                Some(watcher)
            })
            .collect(),
    )
}
