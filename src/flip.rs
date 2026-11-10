// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 NautiluX

//! Meet Flip controls, reconstructed from USB status and verified on 3564:ff06.
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::Path,
    thread,
    time::Duration,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramingMode {
    Auto,
    CloseUp,
    HalfBody,
    FullBody,
}
impl FramingMode {
    pub const ALL: [Self; 4] = [Self::Auto, Self::CloseUp, Self::HalfBody, Self::FullBody];
}
impl std::fmt::Display for FramingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "Auto framing",
            Self::CloseUp => "Close-up",
            Self::HalfBody => "Half body",
            Self::FullBody => "Full body",
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct State {
    pub auto_framing: bool,
    pub framing_gesture: bool,
    pub framing_mode: FramingMode,
    pub zoom_gesture: bool,
    pub zoom_percent: u16,
}
#[derive(Clone, Copy, Debug)]
pub enum Change {
    AutoFraming(bool),
    FramingGesture(bool),
    FramingMode(FramingMode),
    ZoomGesture(bool),
    ZoomRatio(u16),
}
impl State {
    fn decode(data: &[u8]) -> io::Result<Self> {
        if data.len() != 60 || !(40..=60).contains(&data[0]) {
            return Err(io::Error::other("Unsupported Flip status layout"));
        }
        let word = u16::from_le_bytes([data[38], data[39]]);
        let zoom_percent = word >> 4;
        if !(100..=400).contains(&zoom_percent) {
            return Err(io::Error::other("Unsupported Flip gesture zoom range"));
        }
        Ok(Self {
            auto_framing: data[24] != 0,
            framing_gesture: word & 1 != 0,
            framing_mode: FramingMode::ALL[((word >> 1) & 3) as usize],
            zoom_gesture: word & 8 != 0,
            zoom_percent,
        })
    }
    pub fn changed(mut self, change: Change) -> io::Result<Self> {
        match change {
            Change::AutoFraming(value) => self.auto_framing = value,
            Change::FramingGesture(value) => self.framing_gesture = value,
            Change::FramingMode(value) => self.framing_mode = value,
            Change::ZoomGesture(value) => self.zoom_gesture = value,
            Change::ZoomRatio(value) if (100..=400).contains(&value) => self.zoom_percent = value,
            Change::ZoomRatio(_) => return Err(io::Error::other("Zoom ratio must be 1× to 4×")),
        }
        Ok(self)
    }
    fn payload(self, change: Change) -> [u8; 60] {
        if let Change::AutoFraming(_) = change {
            let mut data = [0; 60];
            data[..2].copy_from_slice(&0x0216u16.to_le_bytes());
            // 0 = standard mode; 1 = group auto framing. Byte 3 is the
            // command source for these modes (0 = normal software control).
            data[2] = self.auto_framing.into();
            return data;
        }
        self.gesture_payload()
    }
    fn gesture_payload(self) -> [u8; 60] {
        let mut data = [0; 60];
        data[..2].copy_from_slice(&0x0520u16.to_le_bytes());
        data[2] = self.framing_gesture.into();
        data[3] = self.framing_mode as u8;
        data[4] = self.zoom_gesture.into();
        data[5..7].copy_from_slice(&self.zoom_percent.to_le_bytes());
        data
    }
}

/// Resolve by device number and USB ancestry, never by a display-name substring.
pub fn is_flip(path: &Path) -> bool {
    let Ok(real) = fs::canonicalize(path) else {
        return false;
    };
    let Some(node) = real.file_name() else {
        return false;
    };
    let Ok(device) = fs::canonicalize(
        Path::new("/sys/class/video4linux")
            .join(node)
            .join("device"),
    ) else {
        return false;
    };
    device.ancestors().any(|parent| {
        fs::read_to_string(parent.join("idVendor")).is_ok_and(|id| id.trim() == "3564")
            && fs::read_to_string(parent.join("idProduct")).is_ok_and(|id| id.trim() == "ff06")
    })
}
#[repr(C)]
struct Query {
    unit: u8,
    selector: u8,
    query: u8,
    size: u16,
    data: *mut u8,
}
fn query(file: &File, request: u8, data: &mut [u8]) -> io::Result<()> {
    let mut control = Query {
        unit: 2,
        selector: 6,
        query: request,
        size: data.len() as u16,
        data: data.as_mut_ptr(),
    };
    let ioctl = (3u32 << 30) | ((size_of::<Query>() as u32) << 16) | (u32::from(b'u') << 8) | 0x21;
    // SAFETY: Query matches uvc_xu_control_query; data and file outlive the ioctl.
    if unsafe { libc::ioctl(file.as_raw_fd(), ioctl as libc::c_ulong, &mut control) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn open(path: &Path) -> io::Result<File> {
    if !is_flip(path) {
        return Err(io::Error::other(
            "Gesture controls require an OBSBOT Meet Flip",
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    let mut length = [0; 2];
    query(&file, 0x85, &mut length)?;
    if u16::from_le_bytes(length) != 60 {
        return Err(io::Error::other("Unsupported Flip extension control size"));
    }
    Ok(file)
}
fn read(file: &File) -> io::Result<State> {
    let mut data = [0; 60];
    query(file, 0x81, &mut data)?;
    State::decode(&data)
}
pub fn snapshot(path: &Path) -> io::Result<State> {
    read(&open(path)?)
}
pub fn apply(path: &Path, change: Change) -> io::Result<State> {
    let file = open(path)?;
    // Always read-modify-write the live state, preserving every unrelated field.
    let current = read(&file)?;
    let desired = current.changed(change)?;
    if desired == current {
        return Ok(current);
    }
    query(&file, 0x01, &mut desired.payload(change))?;
    for _ in 0..10 {
        thread::sleep(Duration::from_millis(50));
        if read(&file)? == desired {
            return Ok(desired);
        }
    }
    Err(io::Error::other(
        "The camera did not confirm the Flip setting. Refresh to read its state.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn status(word: u16) -> [u8; 60] {
        let mut data = [0; 60];
        data[0] = 46;
        data[38..40].copy_from_slice(&word.to_le_bytes());
        data
    }
    #[test]
    fn recorded_flip_states_and_preserving_edits() {
        let original = State::decode(&status(0x0c8d)).unwrap();
        assert_eq!(original.framing_mode, FramingMode::HalfBody);
        assert_eq!(original.zoom_percent, 200);
        let framing_off = original.changed(Change::FramingGesture(false)).unwrap();
        assert_eq!(framing_off, State::decode(&status(0x0c8c)).unwrap());
        let both_off = framing_off.changed(Change::ZoomGesture(false)).unwrap();
        assert_eq!(both_off, State::decode(&status(0x0c84)).unwrap());
        assert_eq!(
            &both_off.gesture_payload()[..7],
            &[0x20, 0x05, 0, 2, 0, 200, 0]
        );
        let changed = both_off
            .changed(Change::FramingMode(FramingMode::FullBody))
            .unwrap()
            .changed(Change::ZoomRatio(400))
            .unwrap();
        assert_eq!(
            &changed.gesture_payload()[..7],
            &[0x20, 0x05, 0, 3, 0, 144, 1]
        );
        assert!(changed.gesture_payload()[7..].iter().all(|&byte| byte == 0));
    }
    #[test]
    fn active_framing_is_separate_from_gesture_recognition() {
        let mut data = status(0x0c84);
        data[24] = 1;
        let active = State::decode(&data).unwrap();
        assert!(active.auto_framing);
        assert!(!active.framing_gesture);
        let off = active.changed(Change::AutoFraming(false)).unwrap();
        assert!(!off.auto_framing);
        assert_eq!(off.gesture_payload(), active.gesture_payload());
        let off_payload = off.payload(Change::AutoFraming(false));
        assert_eq!(&off_payload[..4], &[0x16, 0x02, 0, 0]);
        assert!(off_payload[4..].iter().all(|&byte| byte == 0));
        let on_payload = active.payload(Change::AutoFraming(true));
        assert_eq!(&on_payload[..4], &[0x16, 0x02, 1, 0]);
        data[24] = 0;
        assert_eq!(State::decode(&data).unwrap(), off);
        data[24] = 2;
        assert!(State::decode(&data).unwrap().auto_framing);
    }
    #[test]
    fn reject_unknown_status_and_invalid_zoom() {
        assert!(State::decode(&[0; 60]).is_err());
        assert!(State::decode(&status(0)).is_err());
        let state = State::decode(&status(0x0c84)).unwrap();
        assert!(state.changed(Change::ZoomRatio(99)).is_err());
        assert!(state.changed(Change::ZoomRatio(401)).is_err());
        assert!(!is_flip(Path::new("/dev/null")));
        assert_eq!(size_of::<Query>(), 16);
    }
}
