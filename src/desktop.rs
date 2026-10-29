// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 NautiluX

//! Linux desktop integration: session startup, single instance, and tray.
use ksni::blocking::TrayMethods;
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::OpenOptionsExt,
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    sync::mpsc::Sender,
    thread,
};

#[derive(Clone, Debug)]
pub enum Action {
    Show,
    Hide,
    Quit,
    TrayOnline,
    TrayOffline,
}

pub struct Instance {
    _lock: File,
    socket: PathBuf,
}
impl Instance {
    pub fn acquire(sender: Sender<Action>, background: bool) -> io::Result<Option<Self>> {
        // XDG_RUNTIME_DIR is private to the logged-in user. Avoid shared /tmp sockets.
        let dir = env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::other("XDG_RUNTIME_DIR is unavailable"))?;
        let socket = dir.join("camera-control-iced.sock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(dir.join("camera-control-iced.lock"))?;
        // SAFETY: lock owns a valid descriptor, and flock does not retain pointers.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EWOULDBLOCK) {
                return Err(error);
            }
            // Another invocation might still be creating its socket.
            for _ in 0..20 {
                if let Ok(mut stream) = UnixStream::connect(&socket) {
                    stream.write_all(if background { b"ping" } else { b"show" })?;
                    return Ok(None);
                }
                thread::sleep(std::time::Duration::from_millis(50));
            }
            return Err(io::Error::other(
                "Camera Control is already starting; try again.",
            ));
        }
        if socket.exists() {
            fs::remove_file(&socket)?;
        }
        let listener = UnixListener::bind(&socket)?;
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    break;
                };
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(1)));
                let mut bytes = [0; 16];
                if let Ok(count) = stream.read(&mut bytes) {
                    let action = match &bytes[..count] {
                        b"show" => Some(Action::Show),
                        b"hide" => Some(Action::Hide),
                        b"quit" => Some(Action::Quit),
                        _ => None,
                    };
                    if let Some(action) = action {
                        let _ = sender.send(action);
                    }
                }
            }
        });
        Ok(Some(Self {
            _lock: lock,
            socket,
        }))
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket);
    }
}

pub struct Tray {
    pub sender: Sender<Action>,
}
impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "camera-control-iced".into()
    }
    fn title(&self) -> String {
        "Camera Control".into()
    }
    fn activate(&mut self, _: i32, _: i32) {
        let _ = self.sender.send(Action::Show);
    }
    fn icon_name(&self) -> String {
        String::new()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        // A small camera glyph, supplied as ARGB so no icon-theme dependency is needed.
        let mut pixels = vec![0; 32 * 32 * 4];
        for y in 0..32i32 {
            for x in 0..32i32 {
                let body = (3..29).contains(&x) && (9..26).contains(&y);
                let top = (9..18).contains(&x) && (6..10).contains(&y);
                let lens = (x - 16).pow(2) + (y - 17).pow(2) <= 36;
                let color = if lens {
                    [255, 16, 129, 122]
                } else if body || top {
                    [255, 235, 247, 245]
                } else {
                    [0, 0, 0, 0]
                };
                let offset = ((y * 32 + x) * 4) as usize;
                pixels[offset..offset + 4].copy_from_slice(&color);
            }
        }
        vec![ksni::Icon {
            width: 32,
            height: 32,
            data: pixels,
        }]
    }
    fn watcher_online(&self) {
        let _ = self.sender.send(Action::TrayOnline);
    }
    fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
        let _ = self.sender.send(Action::TrayOffline);
        true // Keep the service available if the desktop tray restarts.
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Show Camera Control".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.sender.send(Action::Show);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Hide to tray".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.sender.send(Action::Hide);
                }),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.sender.send(Action::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}
pub fn tray(sender: Sender<Action>) -> Result<ksni::blocking::Handle<Tray>, String> {
    Tray { sender }
        .assume_sni_available(true)
        .spawn()
        .map_err(|error| error.to_string())
}
fn autostart_path() -> io::Result<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .ok_or_else(|| io::Error::other("No configuration directory is available"))?;
    Ok(base.join("autostart/camera-control-iced.desktop"))
}
pub fn starts_at_login() -> bool {
    autostart_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .is_some_and(|text| {
            !text
                .lines()
                .any(|line| line == "Hidden=true" || line == "X-GNOME-Autostart-enabled=false")
        })
}
fn desktop_entry(executable: &Path) -> String {
    let mut argument = String::from("\"");
    for c in executable.to_string_lossy().chars() {
        match c {
            '\\' => argument.push_str(r"\\\\"),
            '"' => argument.push_str(r#"\\\""#),
            '$' => argument.push_str(r"\\$"),
            '`' => argument.push_str(r"\\`"),
            '%' => argument.push_str("%%"),
            '\n' | '\r' => argument.push('_'),
            _ => argument.push(c),
        }
    }
    argument.push('"');
    format!(
        "[Desktop Entry]\nType=Application\nName=Camera Control\nComment=Keep camera color settings applied\nExec={argument} --background\nIcon=camera-control-iced\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    )
}
pub fn set_start_at_login(enabled: bool) -> io::Result<()> {
    let path = autostart_path()?;
    if !enabled {
        return match fs::remove_file(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        };
    }
    fs::create_dir_all(path.parent().unwrap())?;
    let entry = desktop_entry(&env::current_exe()?);
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, entry)?;
    fs::rename(temporary, path)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn autostart_quotes_paths_and_runs_the_tray_app() {
        let entry = desktop_entry(Path::new("/home/test/My Apps/camera%control"));
        assert!(entry.contains("Exec=\"/home/test/My Apps/camera%%control\" --background\n"));
        assert!(!entry.contains("--watch"));
    }
    #[test]
    fn tray_has_accessible_menu_actions_and_valid_argb_icon() {
        use ksni::Tray as _;
        let (tx, rx) = std::sync::mpsc::channel();
        let mut tray = Tray { sender: tx };
        tray.activate(0, 0);
        assert!(matches!(rx.try_recv().unwrap(), Action::Show));
        assert_eq!(tray.menu().len(), 4);
        let icon = tray.icon_pixmap().remove(0);
        assert_eq!(icon.data.len(), (icon.width * icon.height * 4) as usize);
    }
}
