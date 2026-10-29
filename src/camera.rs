// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 NautiluX

//! Standard Linux V4L2 controls. SDK-specific features belong in a separate adapter.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
};

pub const BRIGHTNESS: u32 = 0x00980900;
pub const CONTRAST: u32 = 0x00980901;
pub const SATURATION: u32 = 0x00980902;
pub const AUTO_WB: u32 = 0x0098090c;
pub const TEMPERATURE: u32 = 0x0098091a;
const COLOR_IDS: [u32; 5] = [BRIGHTNESS, CONTRAST, SATURATION, AUTO_WB, TEMPERATURE];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Camera {
    pub path: PathBuf,
    pub name: String,
}
impl std::fmt::Display for Camera {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}
#[derive(Clone, Debug)]
pub struct Control {
    pub id: u32,
    pub minimum: i32,
    pub maximum: i32,
    pub step: i32,
    pub value: i32,
    pub inactive: bool,
}
impl Control {
    pub fn normalize(&self, value: i32) -> i32 {
        let value = value.clamp(self.minimum, self.maximum);
        self.minimum + (value - self.minimum) / self.step.max(1) * self.step.max(1)
    }
}

#[repr(C)]
#[derive(Default)]
struct Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}
#[repr(C)]
#[derive(Default)]
struct QueryControl {
    id: u32,
    kind: u32,
    name: [u8; 32],
    minimum: i32,
    maximum: i32,
    step: i32,
    default_value: i32,
    flags: u32,
    reserved: [u32; 2],
}
#[repr(C)]
struct ControlValue {
    id: u32,
    value: i32,
}

// Linux asm-generic ioctl layout, used by x86_64 and aarch64.
const fn ioctl_code(direction: u32, number: u32, size: usize) -> libc::c_ulong {
    ((direction << 30) | ((size as u32) << 16) | (b'V' as u32) << 8 | number) as libc::c_ulong
}
fn ioctl<T>(file: &File, request: libc::c_ulong, value: &mut T) -> io::Result<()> {
    // SAFETY: each call supplies the matching repr(C) V4L2 structure. The kernel
    // accesses it only during this call, and file owns a valid open descriptor.
    if unsafe { libc::ioctl(file.as_raw_fd(), request, value as *mut T) } < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn open(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
}
fn describe(path: PathBuf) -> io::Result<Camera> {
    let file = open(&path)?;
    let mut cap = Capability::default();
    ioctl(&file, ioctl_code(2, 0, size_of::<Capability>()), &mut cap)?;
    let caps = if cap.capabilities & 0x80000000 != 0 {
        cap.device_caps
    } else {
        cap.capabilities
    };
    if caps & 1 == 0 {
        return Err(io::Error::other("Not a video capture device"));
    }
    let end = cap
        .card
        .iter()
        .position(|b| *b == 0)
        .unwrap_or(cap.card.len());
    Ok(Camera {
        path,
        name: String::from_utf8_lossy(&cap.card[..end]).into_owned(),
    })
}
pub fn discover() -> Vec<Camera> {
    let mut cameras = Vec::new();
    let mut seen = BTreeSet::new();
    for dir in ["/dev/v4l/by-id", "/dev/v4l/by-path", "/dev"] {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if dir == "/dev"
                && !path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("video"))
            {
                continue;
            }
            let Ok(real) = fs::canonicalize(&path) else {
                continue;
            };
            if seen.contains(&real) {
                continue;
            }
            if let Ok(camera) = describe(path) {
                seen.insert(real);
                cameras.push(camera);
            }
        }
    }
    cameras
}
fn read_value(file: &File, id: u32) -> io::Result<i32> {
    let mut control = ControlValue { id, value: 0 };
    ioctl(
        file,
        ioctl_code(3, 27, size_of::<ControlValue>()),
        &mut control,
    )?;
    Ok(control.value)
}
fn write_value(file: &File, id: u32, value: i32) -> io::Result<()> {
    let mut control = ControlValue { id, value };
    ioctl(
        file,
        ioctl_code(3, 28, size_of::<ControlValue>()),
        &mut control,
    )
}
fn query(file: &File, id: u32) -> io::Result<Control> {
    let mut query = QueryControl {
        id,
        ..Default::default()
    };
    ioctl(
        file,
        ioctl_code(3, 36, size_of::<QueryControl>()),
        &mut query,
    )?;
    if query.flags & (1 | 4) != 0 || query.minimum > query.maximum {
        return Err(io::Error::other("Control is disabled or read-only"));
    }
    Ok(Control {
        id,
        minimum: query.minimum,
        maximum: query.maximum,
        step: query.step.max(1),
        value: read_value(file, id)?,
        inactive: query.flags & 16 != 0,
    })
}
pub fn snapshot(path: &Path) -> io::Result<Vec<Control>> {
    let file = open(path)?;
    Ok(COLOR_IDS
        .into_iter()
        .filter_map(|id| query(&file, id).ok())
        .collect())
}
/// Apply only supported requested controls, in dependency order. Unsupported
/// controls are skipped; I/O failures on a known control are reported.
pub fn apply(path: &Path, desired: &BTreeMap<u32, i32>) -> io::Result<usize> {
    let file = open(path)?;
    let mut changes = 0;
    for id in [AUTO_WB, TEMPERATURE, BRIGHTNESS, CONTRAST, SATURATION] {
        let Some(&target) = desired.get(&id) else {
            continue;
        };
        if id == TEMPERATURE && read_value(&file, AUTO_WB).unwrap_or(1) != 0 {
            continue;
        }
        let control = match query(&file, id) {
            Ok(control) => control,
            Err(error) if error.raw_os_error() == Some(libc::EINVAL) => continue,
            Err(error) => return Err(error),
        };
        if control.inactive {
            continue;
        }
        let target = control.normalize(target);
        if control.value != target {
            write_value(&file, id, target)?;
            if read_value(&file, id)? != target {
                return Err(io::Error::other(
                    "Camera did not retain the requested setting",
                ));
            }
            changes += 1;
        }
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v4l2_structs_match_the_linux_abi() {
        assert_eq!(size_of::<Capability>(), 104);
        assert_eq!(size_of::<QueryControl>(), 68);
        assert_eq!(size_of::<ControlValue>(), 8);
        assert_eq!(ioctl_code(3, 27, 8), 0xc008561b);
    }
    #[test]
    fn respects_camera_steps_and_limits() {
        let control = Control {
            id: TEMPERATURE,
            minimum: 2000,
            maximum: 10000,
            step: 100,
            value: 5500,
            inactive: false,
        };
        assert_eq!(control.normalize(5572), 5500);
        assert_eq!(control.normalize(11000), 10000);
        assert_eq!(control.normalize(-5), 2000);
    }
}
