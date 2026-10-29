// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 NautiluX

//! Optional native V4L2 capture. The latest frame replaces its predecessor so a
//! slow UI cannot create an unbounded queue of images.
use iced::widget::image::Handle;
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Cursor},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use v4l::{
    Format, FourCC, buffer::Type, frameinterval::FrameIntervalEnum, framesize::FrameSizeEnum,
    io::traits::CaptureStream, prelude::*, video::Capture,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}
impl std::fmt::Display for Resolution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} × {}", self.width, self.height)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub resolution: Resolution,
    pub fps: u32,
    pub mirror: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            resolution: Resolution {
                width: 1280,
                height: 720,
            },
            fps: 30,
            mirror: false,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Mode {
    pub resolution: Resolution,
    pub frame_rates: Vec<u32>,
}
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub width: u32,
    pub height: u32,
    pub camera_fps: f64,
    pub measured_fps: f64,
}
impl Options {
    pub fn supported(&self, modes: &[Mode]) -> Self {
        let Some(mode) = modes
            .iter()
            .find(|m| m.resolution == self.resolution)
            .or_else(|| {
                modes.iter().min_by_key(|m| {
                    m.resolution.width.abs_diff(self.resolution.width)
                        + m.resolution.height.abs_diff(self.resolution.height)
                })
            })
        else {
            return self.clone();
        };
        // Keep the requested rate, including 60 fps, rather than silently
        // replacing it with an advertised lower rate. Some devices expose
        // incomplete mode tables; the negotiated rate is reported separately.
        let fps = self.fps.clamp(1, 60);
        Self {
            resolution: mode.resolution,
            fps,
            mirror: self.mirror,
        }
    }
}
fn capture_fourcc(device: &Device) -> io::Result<FourCC> {
    let formats = device.enum_formats()?;
    [FourCC::new(b"MJPG"), FourCC::new(b"YUYV")]
        .into_iter()
        .find(|code| formats.iter().any(|format| format.fourcc == *code))
        .ok_or_else(|| io::Error::other("This preview supports MJPEG and YUYV camera streams"))
}
pub fn modes(path: &std::path::Path) -> io::Result<Vec<Mode>> {
    let device = Device::with_path(path)?;
    let fourcc = capture_fourcc(&device)?;
    let sizes = device.enum_framesizes(fourcc)?;
    let mut resolutions = Vec::new();
    for size in sizes {
        match size.size {
            FrameSizeEnum::Discrete(size) => resolutions.push(Resolution {
                width: size.width,
                height: size.height,
            }),
            FrameSizeEnum::Stepwise(size) => {
                for (width, height) in [
                    (640, 480),
                    (1280, 720),
                    (1920, 1080),
                    (2560, 1440),
                    (3840, 2160),
                ] {
                    if (size.min_width..=size.max_width).contains(&width)
                        && (size.min_height..=size.max_height).contains(&height)
                    {
                        resolutions.push(Resolution {
                            width: size.min_width
                                + (width - size.min_width) / size.step_width.max(1)
                                    * size.step_width.max(1),
                            height: size.min_height
                                + (height - size.min_height) / size.step_height.max(1)
                                    * size.step_height.max(1),
                        });
                    }
                }
            }
        }
    }
    resolutions.sort_by_key(|r| (u64::from(r.width) * u64::from(r.height), r.width));
    resolutions.dedup();
    let mut modes = Vec::new();
    for resolution in resolutions {
        let mut frame_rates = Vec::new();
        if let Ok(intervals) =
            device.enum_frameintervals(fourcc, resolution.width, resolution.height)
        {
            for interval in intervals {
                match interval.interval {
                    FrameIntervalEnum::Discrete(fraction) if fraction.numerator > 0 => {
                        let rate = (fraction.denominator as f64 / fraction.numerator as f64).round()
                            as u32;
                        if rate > 0 {
                            frame_rates.push(rate);
                        }
                    }
                    FrameIntervalEnum::Stepwise(range)
                        if range.min.numerator > 0 && range.max.numerator > 0 =>
                    {
                        let min = range.max.denominator as f64 / range.max.numerator as f64;
                        let max = range.min.denominator as f64 / range.min.numerator as f64;
                        frame_rates.extend(
                            [10, 15, 24, 25, 30, 50, 60]
                                .into_iter()
                                .filter(|r| (*r as f64) >= min && (*r as f64) <= max),
                        );
                    }
                    _ => {}
                }
            }
        }
        frame_rates.sort_unstable();
        frame_rates.dedup();
        modes.push(Mode {
            resolution,
            frame_rates,
        });
    }
    Ok(modes)
}
#[derive(Default)]
pub struct State {
    pub frame: Option<Handle>,
    pub error: Option<String>,
    pub info: Option<Info>,
}
pub struct Preview {
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    mirror: Arc<AtomicBool>,
}
impl Preview {
    pub fn start(path: PathBuf, options: Options) -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let mirror = Arc::new(AtomicBool::new(options.mirror));
        let mirror_flag = mirror.clone();
        let output = state.clone();
        let cancel = stop.clone();
        let thread = thread::spawn(move || {
            if let Err(error) = capture(path, &options, &cancel, &mirror_flag, &output)
                && !cancel.load(Ordering::Relaxed)
                && let Ok(mut state) = output.lock()
            {
                state.error = Some(format!(
                    "Preview unavailable: {error}. Close other camera previews and try again."
                ));
            }
        });
        Self {
            state,
            stop,
            thread: Some(thread),
            mirror,
        }
    }
    pub fn set_mirror(&self, mirror: bool) {
        self.mirror.store(mirror, Ordering::Relaxed);
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    pub fn finished(&self) -> bool {
        self.thread
            .as_ref()
            .is_none_or(|thread| thread.is_finished())
    }
    pub fn poll(&self) -> State {
        self.state
            .lock()
            .map(|mut state| State {
                frame: state.frame.take(),
                error: state.error.clone(),
                info: state.info.clone(),
            })
            .unwrap_or_default()
    }
}
impl Drop for Preview {
    fn drop(&mut self) {
        self.stop();
    }
}
fn capture(
    path: PathBuf,
    options: &Options,
    stop: &AtomicBool,
    mirror: &AtomicBool,
    output: &Mutex<State>,
) -> io::Result<()> {
    let device = Device::with_path(&path)
        .map_err(|e| io::Error::new(e.kind(), format!("Opening camera: {e}")))?;
    let fourcc = capture_fourcc(&device)?;
    let selected = options.supported(&modes(&path)?);
    let Resolution { width, height } = selected.resolution;
    let format = device
        .set_format(&Format::new(width, height, fourcc))
        .map_err(|e| io::Error::new(e.kind(), format!("Setting {width}x{height} {fourcc}: {e}")))?;
    if format.width > 8192 || format.height > 8192 {
        return Err(io::Error::other("Unsupported preview dimensions"));
    }
    let mut params = device.params()?;
    params.interval = v4l::Fraction::new(1, selected.fps.max(1));
    let params = match device.set_params(&params) {
        Ok(params) => params,
        Err(error) if error.raw_os_error() == Some(libc::EINVAL) => {
            // A rejected request can fall back to an advertised rate without
            // changing the saved preference or claiming that rate was achieved.
            let available = modes(&path)?;
            let fallback = available
                .iter()
                .find(|mode| mode.resolution == selected.resolution)
                .and_then(|mode| {
                    mode.frame_rates
                        .iter()
                        .copied()
                        .min_by_key(|rate| rate.abs_diff(selected.fps))
                })
                .ok_or(error)?;
            params.interval = v4l::Fraction::new(1, fallback);
            device.set_params(&params)?
        }
        Err(error) => {
            return Err(io::Error::new(
                error.kind(),
                format!("Setting {} fps: {error}", selected.fps),
            ));
        }
    };
    let camera_fps =
        f64::from(params.interval.denominator) / f64::from(params.interval.numerator.max(1));
    let mut info = Info {
        width: format.width,
        height: format.height,
        camera_fps,
        measured_fps: 0.0,
    };
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    let mut stream = MmapStream::with_buffers(&device, Type::VideoCapture, 3)
        .map_err(|e| io::Error::new(e.kind(), format!("Allocating capture buffers: {e}")))?;
    // Some cameras need over a second to wake their sensor. A timed-out
    // next() must terminate this stream: retrying next() would requeue an
    // already queued buffer in v4l 0.14.
    stream.set_timeout(Duration::from_secs(2));
    let mut sample_start = Instant::now();
    let mut sample_frames = 0;
    while !stop.load(Ordering::Relaxed) {
        let (buffer, metadata) = match stream.next() {
            Ok(frame) => frame,

            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!("Reading a frame: {error}"),
                ));
            }
        };
        let data = &buffer[..(metadata.bytesused as usize).min(buffer.len())];
        let (width, height, mut rgba) = if format.fourcc == FourCC::new(b"MJPG") {
            decode_jpeg(data)?
        } else if format.fourcc == FourCC::new(b"YUYV") {
            (
                format.width,
                format.height,
                decode_yuyv(data, format.width, format.height, format.stride)?,
            )
        } else {
            return Err(io::Error::other(
                "Camera negotiated an unsupported preview format",
            ));
        };
        if mirror.load(Ordering::Relaxed) {
            mirror_rgba(&mut rgba, width);
        }
        sample_frames += 1;
        if sample_start.elapsed() >= Duration::from_secs(1) {
            info.measured_fps = sample_frames as f64 / sample_start.elapsed().as_secs_f64();
            sample_start = Instant::now();
            sample_frames = 0;
        }
        if let Ok(mut state) = output.lock() {
            state.frame = Some(Handle::from_rgba(width, height, rgba));
            state.info = Some(info.clone());
        }
    }
    Ok(())
}
fn decode_jpeg(bytes: &[u8]) -> io::Result<(u32, u32, Vec<u8>)> {
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    let pixels = decoder.decode().map_err(io::Error::other)?;
    let info = decoder
        .info()
        .ok_or_else(|| io::Error::other("JPEG has no dimensions"))?;
    let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
    match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            for rgb in pixels.chunks_exact(3) {
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        jpeg_decoder::PixelFormat::L8 => {
            for value in pixels {
                rgba.extend_from_slice(&[value, value, value, 255]);
            }
        }
        _ => return Err(io::Error::other("Unsupported JPEG color format")),
    }
    if rgba.len() != usize::from(info.width) * usize::from(info.height) * 4 {
        return Err(io::Error::other(
            "Decoded frame size does not match its dimensions",
        ));
    }
    Ok((u32::from(info.width), u32::from(info.height), rgba))
}
fn decode_yuyv(bytes: &[u8], width: u32, height: u32, stride: u32) -> io::Result<Vec<u8>> {
    let row_size = width as usize * 2;
    let stride = (stride as usize).max(row_size);
    if !width.is_multiple_of(2) || bytes.len() < stride * height as usize {
        return Err(io::Error::other("Incomplete YUYV frame"));
    }
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for row in bytes.chunks_exact(stride).take(height as usize) {
        for pair in row[..row_size].chunks_exact(4) {
            let u = i32::from(pair[1]) - 128;
            let v = i32::from(pair[3]) - 128;
            for y in [pair[0], pair[2]] {
                let y = (i32::from(y) - 16).max(0);
                rgba.extend_from_slice(&[
                    ((298 * y + 409 * v + 128) >> 8).clamp(0, 255) as u8,
                    ((298 * y - 100 * u - 208 * v + 128) >> 8).clamp(0, 255) as u8,
                    ((298 * y + 516 * u + 128) >> 8).clamp(0, 255) as u8,
                    255,
                ]);
            }
        }
    }
    Ok(rgba)
}
fn mirror_rgba(pixels: &mut [u8], width: u32) {
    if width == 0 {
        return;
    }
    for row in pixels.chunks_exact_mut(width as usize * 4) {
        for x in 0..width as usize / 2 {
            let opposite = (width as usize - 1 - x) * 4;
            let (left, right) = row.split_at_mut(opposite);
            left[x * 4..x * 4 + 4].swap_with_slice(&mut right[..4]);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn yuyv_decoding_handles_padding_and_neutral_colors() {
        let rgba = decode_yuyv(&[16, 128, 235, 128, 99, 99], 2, 1, 6).unwrap();
        assert_eq!(rgba, [0, 0, 0, 255, 255, 255, 255, 255]);
        assert!(decode_yuyv(&[16, 128], 2, 1, 4).is_err());
    }
    #[test]
    fn mirroring_swaps_pixels_inside_each_row_and_is_reversible() {
        let original = vec![1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255];
        let mut frame = original.clone();
        mirror_rgba(&mut frame, 2);
        assert_eq!(
            frame,
            [4, 5, 6, 255, 1, 2, 3, 255, 10, 11, 12, 255, 7, 8, 9, 255]
        );
        mirror_rgba(&mut frame, 2);
        assert_eq!(frame, original);
    }
    #[test]
    fn sixty_fps_request_is_preserved_when_camera_advertises_less() {
        let modes = vec![
            Mode {
                resolution: Resolution {
                    width: 1280,
                    height: 720,
                },
                frame_rates: vec![24, 30, 60],
            },
            Mode {
                resolution: Resolution {
                    width: 1920,
                    height: 1080,
                },
                frame_rates: vec![24, 30],
            },
        ];
        let requested = Options {
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            fps: 60,
            mirror: true,
        };
        let supported = requested.supported(&modes);
        assert_eq!(supported.fps, 60);
        assert!(supported.mirror);
        assert_eq!(supported.resolution, requested.resolution);
    }
    #[test]
    #[ignore = "requires CAMERA_CONTROL_TEST_CAMERA and exclusive capture access"]
    fn hardware_preview_releases_camera_for_a_second_capture() {
        let path =
            std::env::var_os("CAMERA_CONTROL_TEST_CAMERA").expect("set CAMERA_CONTROL_TEST_CAMERA");
        for options in [
            Options::default(),
            Options {
                resolution: Resolution {
                    width: 1920,
                    height: 1080,
                },
                fps: 24,
                mirror: true,
            },
        ] {
            let expected = options.supported(&modes(&PathBuf::from(&path)).unwrap());
            let preview = Preview::start(PathBuf::from(&path), options);
            let start = Instant::now();
            loop {
                let state = preview.poll();
                assert!(state.error.is_none(), "{:?}", state.error);
                if let Some(Handle::Rgba {
                    width,
                    height,
                    pixels,
                    ..
                }) = state.frame
                {
                    assert_eq!(
                        (width, height),
                        (expected.resolution.width, expected.resolution.height)
                    );
                    assert_eq!(pixels.len(), width as usize * height as usize * 4);
                    break;
                }
                assert!(
                    start.elapsed() < Duration::from_secs(10),
                    "No preview frame arrived"
                );
                thread::sleep(Duration::from_millis(50));
            }
            let sample_start = Instant::now();
            let mut info = None;
            while sample_start.elapsed() < Duration::from_secs(3) {
                let state = preview.poll();
                assert!(state.error.is_none(), "{:?}", state.error);
                if let Some(status) = state.info {
                    info = Some(status);
                }
                thread::sleep(Duration::from_millis(8));
            }
            let info = info.unwrap();
            println!(
                "{}x{}: requested {} fps; negotiated {:.2}; decoded {:.2}",
                info.width, info.height, expected.fps, info.camera_fps, info.measured_fps
            );
            assert!((info.camera_fps - expected.fps as f64).abs() < 1.0);
            assert!(info.measured_fps >= expected.fps as f64 * 0.5);
            preview.set_mirror(false);
            preview.stop();
            let start = Instant::now();
            while !preview.finished() {
                assert!(
                    start.elapsed() < Duration::from_secs(3),
                    "Capture did not stop"
                );
                thread::sleep(Duration::from_millis(20));
            }
        }
    }
}
