// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 NautiluX

use crate::{
    camera::{self, Camera, Control},
    settings::Profile,
};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};
pub enum Command {
    Scan,
    Select(Camera, Profile, u64),
    Update(Profile, u64, bool),
    Flip(PathBuf, crate::flip::Change, u64),
    Stop,
}
pub enum Event {
    Cameras(Vec<Camera>),
    FlipSnapshot {
        path: PathBuf,
        revision: u64,
        result: Result<crate::flip::State, String>,
        write_error: Option<String>,
    },
    PreviewModes {
        path: PathBuf,
        result: Result<Vec<crate::preview::Mode>, String>,
    },
    Snapshot {
        path: PathBuf,
        revision: u64,
        result: Result<Vec<Control>, String>,
        corrections: usize,
    },
}
pub struct Worker {
    pub commands: Sender<Command>,
    pub events: Receiver<Event>,
}
impl Worker {
    pub fn start() -> Self {
        let (commands, commands_rx) = mpsc::channel();
        let (events_tx, events) = mpsc::channel();
        thread::spawn(move || {
            let mut selected: Option<Camera> = None;
            let mut profile = Profile::default();
            let mut revision = 0;
            let mut dirty = false;
            let mut flip_write_error = None;
            let mut corrections = 0;
            let mut next_check = Instant::now();
            let mut next_scan = Instant::now() + Duration::from_secs(3);
            let _ = events_tx.send(Event::Cameras(camera::discover()));
            loop {
                match commands_rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Ok(Command::Scan) => {
                        let _ = events_tx.send(Event::Cameras(camera::discover()));
                    }
                    Ok(Command::Select(camera, desired, generation)) => {
                        selected = Some(camera);
                        flip_write_error = None;
                        profile = desired;
                        revision = generation;
                        corrections = 0;
                        if let Some(camera) = &selected {
                            let result =
                                crate::preview::modes(&camera.path).map_err(|e| e.to_string());
                            let _ = events_tx.send(Event::PreviewModes {
                                path: camera.path.clone(),
                                result,
                            });
                        }
                        next_check = Instant::now();
                        dirty = false;
                    }
                    Ok(Command::Update(desired, generation, apply)) => {
                        profile = desired;
                        revision = generation;
                        dirty = apply;
                        next_check = Instant::now();
                    }
                    Ok(Command::Flip(path, change, generation)) => {
                        if selected.as_ref().is_some_and(|camera| camera.path == path) {
                            revision = generation;
                            flip_write_error = crate::flip::apply(&path, change)
                                .err()
                                .map(|error| error.to_string());
                            next_check = Instant::now();
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if Instant::now() >= next_scan {
                    let _ = events_tx.send(Event::Cameras(camera::discover()));
                    if let Some(camera) = &selected {
                        let result = crate::preview::modes(&camera.path).map_err(|e| e.to_string());
                        let _ = events_tx.send(Event::PreviewModes {
                            path: camera.path.clone(),
                            result,
                        });
                    }
                    next_scan = Instant::now() + Duration::from_secs(3);
                }
                if dirty || Instant::now() >= next_check {
                    if let Some(camera) = &selected {
                        if crate::flip::is_flip(&camera.path) {
                            let _ = events_tx.send(Event::FlipSnapshot {
                                path: camera.path.clone(),
                                revision,
                                result: crate::flip::snapshot(&camera.path)
                                    .map_err(|error| error.to_string()),
                                write_error: flip_write_error.clone(),
                            });
                        }
                        let result = if dirty || profile.watch {
                            camera::apply(&camera.path, &profile.desired).map(|count| {
                                corrections += count;
                            })
                        } else {
                            Ok(())
                        };
                        let result = result
                            .and_then(|_| camera::snapshot(&camera.path))
                            .map_err(|e| e.to_string());
                        if events_tx
                            .send(Event::Snapshot {
                                path: camera.path.clone(),
                                revision,
                                result,
                                corrections,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    dirty = false;
                    next_check = Instant::now() + Duration::from_millis(500);
                }
            }
        });
        Self { commands, events }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
    }
}
