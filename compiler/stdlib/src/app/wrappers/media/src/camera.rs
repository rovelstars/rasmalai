//! Camera capture. V4L2 on Linux, synthetic frames for headless runs.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use v4l::buffer::Type;
use v4l::io::mmap::Stream;
use v4l::io::traits::{CaptureStream, Stream as StreamTrait};
use v4l::prelude::Device;
use v4l::video::Capture;
use v4l::{Format, FourCC};

use crate::frame;

const QUEUE_LIMIT: usize = 2;
const STUB_FPS: i64 = 10;
const CAPTURE_TIMEOUT: Duration = Duration::from_millis(1000);

// No MJPEG decoder is linked, so MJPG is never selected.
const MJPEG_DECODER: bool = false;

type Queue = Arc<(Mutex<VecDeque<Vec<u8>>>, Condvar)>;

pub struct Camera {
    width: i64,
    height: i64,
    fps_milli: i64,
    produced: Arc<AtomicI64>,
    queue: Queue,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Camera {
    pub fn open(device: &str, width: i32, height: i32) -> Option<Camera> {
        let width = if width > 0 { width as u32 } else { 320 };
        let height = if height > 0 { height as u32 } else { 240 };
        let requested = std::env::var("RNX_CAMERA").ok();
        let stub =
            requested.as_deref() == Some("stub") || (requested.is_some() && device.is_empty());
        if stub {
            return Camera::synthetic(width, height);
        }
        let path = if device.is_empty() {
            "/dev/video0"
        } else {
            device
        };
        Camera::capture(path, width, height)
    }

    pub fn info(&self, lane: i32) -> i64 {
        match lane {
            0 => self.width,
            1 => self.height,
            2 => self.fps_milli,
            3 => self.produced.load(Ordering::Relaxed),
            _ => -1,
        }
    }

    pub fn try_next(&mut self) -> Option<Vec<u8>> {
        let (lock, cvar) = &*self.queue;
        let mut queue = lock.lock().ok()?;
        let frame = queue.pop_front();
        if frame.is_some() {
            cvar.notify_all();
        }
        frame
    }

    pub fn pending(&self) -> i64 {
        match self.queue.0.lock() {
            Ok(queue) => queue.len() as i64,
            Err(_) => 0,
        }
    }

    fn synthetic(width: u32, height: u32) -> Option<Camera> {
        let queue: Queue = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let produced = Arc::new(AtomicI64::new(0));

        let worker_queue = Arc::clone(&queue);
        let worker_stop = Arc::clone(&stop);
        let worker_produced = Arc::clone(&produced);
        let worker = thread::Builder::new()
            .name("rnx-camera-stub".to_string())
            .spawn(move || {
                let period = Duration::from_millis(1000 / STUB_FPS as u64);
                let mut deadline = Instant::now();
                let mut tick: i64 = 0;
                while !worker_stop.load(Ordering::Relaxed) {
                    let pixels = synthetic_pixels(width as usize, height as usize, tick);
                    let packed = frame::pack(width, height, &pixels);
                    if !push(&worker_queue, &worker_stop, packed) {
                        break;
                    }
                    worker_produced.fetch_add(1, Ordering::Relaxed);
                    tick = tick.wrapping_add(1);
                    deadline += period;
                    pause_until(&worker_stop, deadline);
                }
            })
            .ok()?;

        Some(Camera {
            width: width as i64,
            height: height as i64,
            fps_milli: STUB_FPS * 1000,
            produced,
            queue,
            stop,
            worker: Some(worker),
        })
    }

    fn capture(path: &str, width: u32, height: u32) -> Option<Camera> {
        let device = Device::with_path(path).ok()?;
        let fourcc = choose_format(&device)?;
        let format = device
            .set_format(&Format::new(width, height, fourcc))
            .ok()?;
        let width = format.width;
        let height = format.height;
        let stride = format.stride as usize;
        let fps_milli = fps_milli(&device);

        let queue: Queue = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let produced = Arc::new(AtomicI64::new(0));

        let worker_queue = Arc::clone(&queue);
        let worker_stop = Arc::clone(&stop);
        let worker_produced = Arc::clone(&produced);
        let worker = thread::Builder::new()
            .name("rnx-camera".to_string())
            .spawn(move || {
                run_capture(
                    device,
                    width,
                    height,
                    stride,
                    fourcc,
                    worker_queue,
                    worker_stop,
                    worker_produced,
                );
            })
            .ok()?;

        Some(Camera {
            width: width as i64,
            height: height as i64,
            fps_milli,
            produced,
            queue,
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.queue.1.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn choose_format(device: &Device) -> Option<FourCC> {
    let formats = device.enum_formats().ok()?;
    let mut order = vec![FourCC::new(b"YUYV"), FourCC::new(b"RGB3")];
    if MJPEG_DECODER {
        order.push(FourCC::new(b"MJPG"));
    }
    order
        .into_iter()
        .find(|fourcc| formats.iter().any(|fmt| fmt.fourcc == *fourcc))
}

fn fps_milli(device: &Device) -> i64 {
    match device.params() {
        Ok(params) => {
            let numerator = params.interval.numerator as i64;
            let denominator = params.interval.denominator as i64;
            if numerator == 0 || denominator == 0 {
                30_000
            } else {
                denominator * 1000 / numerator
            }
        }
        Err(_) => 30_000,
    }
}

fn run_capture(
    device: Device,
    width: u32,
    height: u32,
    stride: usize,
    fourcc: FourCC,
    queue: Queue,
    stop: Arc<AtomicBool>,
    produced: Arc<AtomicI64>,
) {
    let Ok(mut stream) = Stream::with_buffers(&device, Type::VideoCapture, 4) else {
        return;
    };
    stream.set_timeout(CAPTURE_TIMEOUT);
    let width = width as usize;
    let height = height as usize;

    while !stop.load(Ordering::Relaxed) {
        // A timeout leaves the buffer queued, so another next() would fail with
        // EBUSY. A stalled device ends the worker instead.
        let Ok((data, _meta)) = stream.next() else {
            break;
        };
        let Some(pixels) = convert(fourcc, data, width, height, stride) else {
            break;
        };
        let packed = frame::pack(width as u32, height as u32, &pixels);
        if !push(&queue, &stop, packed) {
            break;
        }
        produced.fetch_add(1, Ordering::Relaxed);
    }

    let _ = stream.stop();
}

fn push(queue: &Queue, stop: &AtomicBool, frame: Vec<u8>) -> bool {
    let (lock, cvar) = &**queue;
    let mut guard = match lock.lock() {
        Ok(guard) => guard,
        Err(_) => return false,
    };
    while guard.len() >= QUEUE_LIMIT {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        guard = match cvar.wait_timeout(guard, Duration::from_millis(20)) {
            Ok((guard, _)) => guard,
            Err(_) => return false,
        };
    }
    if stop.load(Ordering::Relaxed) {
        return false;
    }
    guard.push_back(frame);
    cvar.notify_all();
    true
}

fn pause_until(stop: &AtomicBool, deadline: Instant) {
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        thread::sleep((deadline - now).min(Duration::from_millis(10)));
    }
}

fn convert(
    fourcc: FourCC,
    data: &[u8],
    width: usize,
    height: usize,
    stride: usize,
) -> Option<Vec<u8>> {
    if width == 0 || height == 0 {
        return None;
    }
    if fourcc == FourCC::new(b"YUYV") {
        Some(yuyv_to_rgba(data, width, height, stride))
    } else if fourcc == FourCC::new(b"RGB3") {
        Some(rgb24_to_rgba(data, width, height, stride))
    } else {
        None
    }
}

fn yuyv_to_rgba(data: &[u8], width: usize, height: usize, stride: usize) -> Vec<u8> {
    let stride = stride.max(width * 2);
    let mut out = vec![0u8; width * height * 4];
    for y in 0..height {
        let Some(row) = data.get(y * stride..) else {
            break;
        };
        for pair in 0..(width + 1) / 2 {
            let base = pair * 4;
            if base + 4 > row.len() {
                break;
            }
            let y0 = row[base] as i32;
            let u = row[base + 1] as i32 - 128;
            let y1 = row[base + 2] as i32;
            let v = row[base + 3] as i32 - 128;
            let x = pair * 2;
            write_yuv(&mut out, (y * width + x) * 4, y0, u, v);
            if x + 1 < width {
                write_yuv(&mut out, (y * width + x + 1) * 4, y1, u, v);
            }
        }
    }
    out
}

fn write_yuv(out: &mut [u8], offset: usize, y: i32, u: i32, v: i32) {
    let r = y + (v * 359) / 256;
    let g = y - (u * 88) / 256 - (v * 183) / 256;
    let b = y + (u * 454) / 256;
    out[offset] = clamp_byte(r);
    out[offset + 1] = clamp_byte(g);
    out[offset + 2] = clamp_byte(b);
    out[offset + 3] = 255;
}

fn rgb24_to_rgba(data: &[u8], width: usize, height: usize, stride: usize) -> Vec<u8> {
    let stride = stride.max(width * 3);
    let mut out = vec![0u8; width * height * 4];
    for y in 0..height {
        let Some(row) = data.get(y * stride..) else {
            break;
        };
        for x in 0..width {
            let source = x * 3;
            if source + 3 > row.len() {
                break;
            }
            let target = (y * width + x) * 4;
            out[target] = row[source];
            out[target + 1] = row[source + 1];
            out[target + 2] = row[source + 2];
            out[target + 3] = 255;
        }
    }
    out
}

fn clamp_byte(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

fn synthetic_pixels(width: usize, height: usize, tick: i64) -> Vec<u8> {
    let mut pixels = vec![0u8; width * height * 4];
    let phase = (tick as usize).wrapping_mul(3);

    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) * 4;
            pixels[index] = ((x + phase) % 256) as u8;
            pixels[index + 1] = ((y + phase * 2) % 256) as u8;
            pixels[index + 2] = (((x + y) / 2 + phase) % 256) as u8;
            pixels[index + 3] = 255;
        }
    }

    let rect_w = (width / 3).max(4).min(width);
    let rect_h = (height / 4).max(4).min(height);
    let span = width.saturating_sub(rect_w).max(1);
    let left = (tick as usize).wrapping_mul(2) % span;
    let top = height.saturating_sub(rect_h) / 2;
    for y in top..(top + rect_h).min(height) {
        for x in left..(left + rect_w).min(width) {
            let index = (y * width + x) * 4;
            pixels[index] = 255;
            pixels[index + 1] = 200;
            pixels[index + 2] = 40;
            pixels[index + 3] = 255;
        }
    }

    let tile = (width / 16).clamp(2, 8).min(width.max(1));
    if tile > 0 {
        for bit in 0..16 {
            let value = if (tick >> bit) & 1 == 1 { 255 } else { 16 };
            let left = (bit % 8) * tile;
            let top = (bit / 8) * tile;
            for y in top..(top + tile).min(height) {
                for x in left..(left + tile).min(width) {
                    let index = (y * width + x) * 4;
                    pixels[index] = value;
                    pixels[index + 1] = value;
                    pixels[index + 2] = value;
                    pixels[index + 3] = 255;
                }
            }
        }
    }

    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn restore(key: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    fn poll_frame(camera: &mut Camera) -> Option<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Some(frame) = camera.try_next() {
                return Some(frame);
            }
            thread::sleep(Duration::from_millis(5));
        }
        None
    }

    #[test]
    fn stub_camera_produces_moving_frames() {
        let saved = std::env::var_os("RNX_CAMERA");
        std::env::set_var("RNX_CAMERA", "stub");
        let camera = Camera::open("", 64, 48);
        restore("RNX_CAMERA", saved);
        let mut camera = camera.expect("synthetic camera");

        assert_eq!(camera.info(0), 64);
        assert_eq!(camera.info(1), 48);
        assert_eq!(camera.info(2), 10_000);

        let first = poll_frame(&mut camera).expect("first frame");
        let (width, height, length) = frame::parse_header(&first).expect("RNXR header");
        assert_eq!((width, height), (64, 48));
        assert_eq!(length, 64 * 48 * 4);

        let pixels = &first[frame::HEADER..frame::HEADER + length];
        let origin = &pixels[0..4];
        assert!(pixels.chunks_exact(4).any(|pixel| pixel != origin));

        let before = camera.info(3);
        assert!(before >= 1);
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut second = None;
        while Instant::now() < deadline && camera.info(3) <= before {
            thread::sleep(Duration::from_millis(5));
        }
        if camera.info(3) > before {
            second = poll_frame(&mut camera);
        }
        assert!(camera.info(3) > before);
        assert!(camera.pending() >= 0);
        let second = second.expect("second frame");
        assert_ne!(second, first);
    }
}
