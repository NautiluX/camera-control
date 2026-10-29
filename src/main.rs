// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 NautiluX

mod camera;
mod desktop;
mod preview;
mod settings;
mod ui;
mod worker;
use camera::{AUTO_WB, BRIGHTNESS, CONTRAST, Camera, Control, SATURATION, TEMPERATURE};
use iced::{Element, Subscription, Task, window};
use settings::{Profile, Settings};
use std::{
    sync::mpsc::{self, Receiver},
    time::Duration,
};
use worker::{Command, Event, Worker};

fn main() -> iced::Result {
    match std::env::args().nth(1).as_deref() {
        Some("--version") | Some("-V") => {
            println!("Camera Control {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--list") => {
            for camera in camera::discover() {
                println!("{}\n  {}", camera.name, camera.path.display());
                match camera::snapshot(&camera.path) {
                    Ok(controls) => {
                        for c in controls {
                            println!(
                                "  {}: {} ({}..{}, step {})",
                                label(c.id),
                                c.value,
                                c.minimum,
                                c.maximum,
                                c.step
                            );
                        }
                    }
                    Err(error) => eprintln!("  {error}"),
                }
            }
            return Ok(());
        }
        Some("--watch") => {
            let mut last_error = String::new();
            loop {
                let result = Settings::load().and_then(|settings| {
                    for (path, profile) in settings.cameras.iter().filter(|(_, p)| p.watch) {
                        camera::apply(path, &profile.desired)?;
                    }
                    Ok(())
                });
                let error = result.err().map(|e| e.to_string()).unwrap_or_default();
                if !error.is_empty() && error != last_error {
                    eprintln!("{error}");
                }
                last_error = error;
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        Some("--help") | Some("-h") => {
            println!(
                "Camera Control\n\n  camera-control           Open the Iced interface\n  camera-control --list    List cameras and read controls without changing them\n  camera-control --watch   Enforce saved enabled profiles without a window\n\nNo OBSBOT SDK is required for standard color controls."
            );
            return Ok(());
        }
        _ => {}
    }
    iced::daemon(App::boot, App::update, App::view)
        .title("Camera Control")
        .default_font(iced::Font::with_name("Noto Sans"))
        .theme(|_: &App, _: window::Id| ui::theme())
        .subscription(|app: &App| {
            Subscription::batch([
                iced::time::every(Duration::from_millis(
                    if app.preview_enabled || app.preview_restart {
                        16
                    } else {
                        100
                    },
                ))
                .map(|_| Message::Tick),
                window::close_requests().map(Message::CloseRequested),
            ])
        })
        .run()
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    Refresh,
    Selected(Camera),
    Value(u32, i32),
    Watch(bool),
    Preset(i32),
    Preview(bool),
    PreviewFrameReady(
        u64,
        Result<iced_runtime::image::Allocation, iced_runtime::image::Error>,
    ),
    PreviewResolution(preview::Resolution),
    PreviewFrameRate(u32),
    PreviewMirror(bool),
    StartAtLogin(bool),
    Hide,
    Quit,
    CloseRequested(window::Id),
    WindowOpened(window::Id),
}
struct App {
    worker: Worker,
    settings: Settings,
    cameras: Vec<Camera>,
    selected: Option<Camera>,
    controls: Vec<Control>,
    revision: u64,
    status: String,
    error: Option<String>,
    config_error: Option<String>,
    corrections: usize,
    desktop_events: Receiver<desktop::Action>,
    _instance: Option<desktop::Instance>,
    tray: Option<ksni::blocking::Handle<desktop::Tray>>,
    tray_ready: bool,
    window: Option<window::Id>,
    start_at_login: bool,
    desktop_error: Option<String>,
    duplicate: bool,
    preview: Option<preview::Preview>,
    preview_enabled: bool,
    preview_frame: Option<iced::widget::image::Handle>,
    preview_allocation: Option<iced_runtime::image::Allocation>,
    preview_generation: u64,
    preview_upload_pending: bool,
    preview_error: Option<String>,
    preview_modes: Vec<preview::Mode>,
    preview_info: Option<preview::Info>,
    preview_restart: bool,
}
impl App {
    fn boot() -> (Self, Task<Message>) {
        let mut app = Self::new();
        if app.duplicate {
            return (app, iced::exit());
        }
        let background = std::env::args().any(|argument| argument == "--background");
        let task = if background && app.tray.is_some() {
            Task::none()
        } else {
            app.show()
        };
        (app, task)
    }
    fn show(&mut self) -> Task<Message> {
        if let Some(id) = self.window {
            return window::gain_focus(id);
        }
        let (id, task) = window::open(window::Settings {
            size: iced::Size::new(1010.0, 760.0),
            min_size: Some(iced::Size::new(850.0, 670.0)),
            exit_on_close_request: false,
            ..Default::default()
        });
        self.window = Some(id);
        task.map(Message::WindowOpened)
    }
    fn start_preview(&mut self) {
        if self.preview.is_some() {
            return;
        }
        if let Some(camera) = &self.selected {
            self.preview_error = None;
            self.preview_frame = None;
            self.preview_allocation = None;
            self.preview_generation += 1;
            self.preview_upload_pending = false;
            self.preview_info = None;
            self.preview = Some(preview::Preview::start(
                camera.path.clone(),
                self.profile().preview,
            ));
            self.preview_enabled = true;
        }
    }
    fn restart_preview(&mut self) {
        if !self.preview_enabled && !self.preview_restart {
            return;
        }
        if let Some(preview) = &self.preview {
            preview.stop();
        }
        self.preview_enabled = false;
        self.preview_generation += 1;
        self.preview_upload_pending = false;
        self.preview_frame = None;
        self.preview_allocation = None;
        self.preview_info = None;
        self.preview_restart = true;
    }
    fn stop_preview(&mut self) {
        if let Some(preview) = &self.preview {
            preview.stop();
        }
        self.preview_enabled = false;
        self.preview_generation += 1;
        self.preview_upload_pending = false;
        self.preview_frame = None;
        self.preview_allocation = None;
        self.preview_info = None;
        self.preview_restart = false;
    }
    fn hide(&mut self) -> Task<Message> {
        if !self.tray_ready {
            self.desktop_error = Some(
                "Tray unavailable. Keep this window open, or use Quit to stop the app.".into(),
            );
            return Task::none();
        }
        self.stop_preview();
        self.window
            .take()
            .map(window::close)
            .unwrap_or_else(Task::none)
    }
    fn new() -> Self {
        let (settings, config_error) = match Settings::load() {
            Ok(settings) => (settings, None),
            Err(error) => (
                Settings::default(),
                Some(format!("Could not read saved settings: {error}")),
            ),
        };
        let (desktop_tx, desktop_events) = mpsc::channel();
        let background = std::env::args().any(|argument| argument == "--background");
        let (instance, duplicate, mut desktop_error) =
            match desktop::Instance::acquire(desktop_tx.clone(), background) {
                Ok(Some(instance)) => (Some(instance), false, None),
                Ok(None) => (None, true, None),
                Err(error) => (
                    None,
                    true,
                    Some(format!("Could not start desktop integration: {error}")),
                ),
            };
        if let Some(error) = &desktop_error {
            eprintln!("{error}");
        }
        let tray = if duplicate {
            None
        } else {
            match desktop::tray(desktop_tx) {
                Ok(tray) => Some(tray),
                Err(error) => {
                    desktop_error = Some(format!("Tray unavailable: {error}"));
                    None
                }
            }
        };
        let tray_ready = tray.is_some();
        Self {
            desktop_events,
            _instance: instance,
            tray,
            duplicate,
            tray_ready,
            window: None,
            start_at_login: desktop::starts_at_login(),
            desktop_error,
            preview: None,
            preview_enabled: false,
            preview_frame: None,
            preview_allocation: None,
            preview_generation: 0,
            preview_upload_pending: false,
            preview_error: None,
            preview_modes: Vec::new(),
            preview_info: None,
            preview_restart: false,
            worker: Worker::start(),
            settings,
            cameras: vec![],
            selected: None,
            controls: vec![],
            revision: 0,
            status: "Looking for cameras…".into(),
            error: None,
            config_error,
            corrections: 0,
        }
    }
    fn profile(&self) -> Profile {
        self.selected
            .as_ref()
            .and_then(|c| self.settings.cameras.get(&c.path))
            .cloned()
            .unwrap_or_default()
    }
    fn persist(&mut self) {
        // Do not overwrite an unreadable configuration. Let the user repair it first.
        if self.config_error.is_some() {
            return;
        }
        if let Err(error) = self.settings.save() {
            self.config_error = Some(format!("Could not save settings: {error}"));
        }
    }
    fn select(&mut self, camera: Camera) {
        self.stop_preview();
        self.preview_modes.clear();
        self.revision += 1;
        self.controls.clear();
        self.error = None;
        self.corrections = 0;
        self.settings.selected = Some(camera.path.clone());
        self.selected = Some(camera.clone());
        let _ = self
            .worker
            .commands
            .send(Command::Select(camera, self.profile(), self.revision));
        self.persist();
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        let mut tasks = Vec::new();
        if matches!(message, Message::Tick) {
            while let Ok(action) = self.desktop_events.try_recv() {
                match action {
                    desktop::Action::Show => tasks.push(self.show()),
                    desktop::Action::Hide => tasks.push(self.hide()),
                    desktop::Action::Quit => return iced::exit(),
                    desktop::Action::TrayOnline => {
                        self.tray_ready = true;
                        self.desktop_error = None;
                    }
                    desktop::Action::TrayOffline => {
                        self.tray_ready = false;
                        self.desktop_error = Some(
                            "No system tray is available. The app will keep a window open.".into(),
                        );
                        if self.window.is_none() {
                            tasks.push(self.show());
                        }
                    }
                }
            }
            if let Some(preview) = &self.preview {
                let state = preview.poll();
                if self.preview_enabled
                    && !self.preview_upload_pending
                    && let Some(frame) = state.frame
                {
                    // Retain the displayed frame until its replacement has been
                    // uploaded. Swapping raw handles first can draw a blank frame.
                    self.preview_upload_pending = true;
                    let generation = self.preview_generation;
                    tasks.push(
                        iced_runtime::image::allocate(frame)
                            .map(move |result| Message::PreviewFrameReady(generation, result)),
                    );
                }
                if let Some(info) = state.info {
                    self.preview_info = Some(info);
                }
                if let Some(error) = state.error {
                    self.preview_error = Some(error);
                }
                if preview.finished() {
                    self.preview = None;
                    self.preview_enabled = false;
                }
            }
            if self.preview_restart && self.preview.is_none() && self.window.is_some() {
                self.preview_restart = false;
                self.start_preview();
            }
        }
        match message {
            Message::Hide => return self.hide(),
            Message::Quit => return iced::exit(),
            Message::CloseRequested(id) if self.window == Some(id) => {
                if !self.tray_ready {
                    return iced::exit();
                }
                return self.hide();
            }
            Message::WindowOpened(id) => self.window = Some(id),
            Message::PreviewFrameReady(generation, result) => {
                if generation != self.preview_generation || !self.preview_enabled {
                    return Task::none();
                }
                self.preview_upload_pending = false;
                match result {
                    Ok(allocation) => {
                        self.preview_frame = Some(allocation.handle().clone());
                        self.preview_allocation = Some(allocation);
                    }
                    Err(error) => {
                        self.stop_preview();
                        self.preview_error =
                            Some(format!("Could not display the preview frame: {error}"));
                    }
                }
            }
            Message::Preview(enabled) => {
                if enabled {
                    self.start_preview();
                } else {
                    self.stop_preview();
                }
            }
            Message::PreviewResolution(resolution) => {
                if let Some(camera) = &self.selected {
                    let profile = self
                        .settings
                        .cameras
                        .entry(camera.path.clone())
                        .or_default();
                    profile.preview.resolution = resolution;
                    profile.preview = profile.preview.supported(&self.preview_modes);
                    self.persist();
                    self.restart_preview();
                }
            }
            Message::PreviewFrameRate(fps) => {
                if let Some(camera) = &self.selected {
                    let profile = self
                        .settings
                        .cameras
                        .entry(camera.path.clone())
                        .or_default();
                    profile.preview.fps = fps;
                    profile.preview = profile.preview.supported(&self.preview_modes);
                    self.persist();
                    self.restart_preview();
                }
            }
            Message::PreviewMirror(mirror) => {
                if let Some(camera) = &self.selected {
                    self.settings
                        .cameras
                        .entry(camera.path.clone())
                        .or_default()
                        .preview
                        .mirror = mirror;
                    if let Some(preview) = &self.preview {
                        preview.set_mirror(mirror);
                    }
                    self.persist();
                }
            }
            Message::StartAtLogin(enabled) => match desktop::set_start_at_login(enabled) {
                Ok(()) => {
                    self.start_at_login = enabled;
                    self.desktop_error = None;
                }
                Err(error) => {
                    self.desktop_error = Some(format!("Could not update startup settings: {error}"))
                }
            },
            Message::Preset(kelvin) => {
                let Some(camera) = &self.selected else {
                    return Task::none();
                };
                let Some(control) = self.controls.iter().find(|c| c.id == TEMPERATURE) else {
                    return Task::none();
                };
                if kelvin < control.minimum || kelvin > control.maximum {
                    return Task::none();
                }
                let profile = self
                    .settings
                    .cameras
                    .entry(camera.path.clone())
                    .or_default();
                profile.desired.insert(AUTO_WB, 0);
                profile
                    .desired
                    .insert(TEMPERATURE, control.normalize(kelvin));
                self.revision += 1;
                let _ = self.worker.commands.send(Command::Update(
                    profile.clone(),
                    self.revision,
                    true,
                ));
                self.persist();
            }
            other => self.update_state(other),
        }
        Task::batch(tasks)
    }
    fn update_state(&mut self, message: Message) {
        match message {
            Message::Tick => {
                while let Ok(event) = self.worker.events.try_recv() {
                    match event {
                        Event::PreviewModes { path, result } => {
                            if self.selected.as_ref().map(|c| &c.path) != Some(&path) {
                                continue;
                            }
                            if let Ok(modes) = result {
                                let profile = self.settings.cameras.entry(path).or_default();
                                let supported = profile.preview.supported(&modes);
                                let changed = supported != profile.preview;
                                profile.preview = supported;
                                self.preview_modes = modes;
                                if changed {
                                    self.persist();
                                    self.restart_preview();
                                }
                            }
                        }
                        Event::Cameras(cameras) => {
                            self.cameras = cameras;
                            if let Some(selected) = &mut self.selected {
                                if let Some(found) =
                                    self.cameras.iter().find(|c| c.path == selected.path)
                                {
                                    selected.name = found.name.clone();
                                } else {
                                    self.cameras.push(selected.clone());
                                }
                            }
                            if self.selected.is_none() {
                                let preferred = if let Some(path) = &self.settings.selected {
                                    Some(
                                        self.cameras
                                            .iter()
                                            .find(|c| &c.path == path)
                                            .cloned()
                                            .unwrap_or(Camera {
                                                path: path.clone(),
                                                name: "Saved camera (disconnected)".into(),
                                            }),
                                    )
                                } else {
                                    self.cameras
                                        .iter()
                                        .find(|c| c.name.contains("OBSBOT"))
                                        .or_else(|| self.cameras.first())
                                        .cloned()
                                };
                                if let Some(camera) = preferred {
                                    self.select(camera);
                                } else {
                                    self.status =
                                        "No cameras found. Connect a camera and refresh.".into();
                                }
                            }
                        }
                        Event::Snapshot {
                            path,
                            revision,
                            result,
                            corrections,
                        } => {
                            if self.selected.as_ref().map(|c| &c.path) != Some(&path)
                                || revision != self.revision
                            {
                                continue;
                            }
                            match result {
                                Ok(controls) => {
                                    let profile = self.settings.cameras.entry(path).or_default();
                                    for control in &controls {
                                        if !profile.watch {
                                            profile.desired.insert(control.id, control.value);
                                        } else {
                                            profile
                                                .desired
                                                .entry(control.id)
                                                .or_insert(control.value);
                                        }
                                    }
                                    self.controls = controls;
                                    self.error = None;
                                    self.corrections = corrections;
                                    self.status = if profile.watch {
                                        "Watching camera settings"
                                    } else {
                                        "Connected"
                                    }
                                    .into();
                                }
                                Err(error) => {
                                    self.error =
                                        Some(format!("Camera unavailable: {error}. Retrying…"));
                                }
                            }
                        }
                    }
                }
            }
            Message::Refresh => {
                let _ = self.worker.commands.send(Command::Scan);
            }
            Message::Selected(camera) => self.select(camera),
            Message::Value(id, value) => {
                let Some(camera) = &self.selected else {
                    return;
                };
                let profile = self
                    .settings
                    .cameras
                    .entry(camera.path.clone())
                    .or_default();
                let value = self
                    .controls
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| c.normalize(value))
                    .unwrap_or(value);
                profile.desired.insert(id, value);
                let profile = profile.clone();
                self.revision += 1;
                let _ = self
                    .worker
                    .commands
                    .send(Command::Update(profile, self.revision, true));
                self.persist();
            }
            Message::Watch(enabled) => {
                let Some(camera) = &self.selected else {
                    return;
                };
                let profile = self
                    .settings
                    .cameras
                    .entry(camera.path.clone())
                    .or_default();
                profile.watch = enabled;
                let profile = profile.clone();
                self.revision += 1;
                let _ = self
                    .worker
                    .commands
                    .send(Command::Update(profile, self.revision, enabled));
                self.persist();
            }
            _ => {}
        }
    }
    fn view(&self, _: window::Id) -> Element<'_, Message> {
        ui::view(self)
    }
}
fn label(id: u32) -> &'static str {
    match id {
        BRIGHTNESS => "Brightness",
        CONTRAST => "Contrast",
        SATURATION => "Saturation",
        AUTO_WB => "Automatic white balance",
        TEMPERATURE => "White balance",
        _ => "Control",
    }
}
