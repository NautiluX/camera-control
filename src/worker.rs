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
    Stop,
}
pub enum Event {
    Cameras(Vec<Camera>),
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
