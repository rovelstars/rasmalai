//! Video decode.
//!
//! A worker thread demuxes the best video stream, decodes packets, scales each
//! frame to RGBA8, and pushes packed RNXR frames into a queue of at most four.
//! The worker blocks while that queue is full and is joined by `Drop`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, Once};
use std::thread::{self, JoinHandle};

use ffmpeg::software::scaling;
use ffmpeg::util::format::Pixel;
use ffmpeg::{codec, format, frame, media};
use ffmpeg_next as ffmpeg;

use crate::frame as rnxr;

const QUEUE_LIMIT: usize = 4;

static FFMPEG_INIT: Once = Once::new();

fn init_ffmpeg() {
    FFMPEG_INIT.call_once(|| {
        let _ = ffmpeg::init();
    });
}

struct Core {
    queue: VecDeque<Vec<u8>>,
    seek_target: i64,
    seek_gen: u64,
}

struct Shared {
    core: Mutex<Core>,
    cv: Condvar,
    stop: AtomicBool,
    width: i64,
    height: i64,
    duration_ms: i64,
    fps_milli: i64,
    has_audio: bool,
}

pub struct Video {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

fn stream_ms(duration: i64, tb: ffmpeg::Rational) -> i64 {
    if duration <= 0 || tb.denominator() <= 0 {
        return 0;
    }
    let value = (duration as i128) * (tb.numerator() as i128) * 1000 / (tb.denominator() as i128);
    value.clamp(0, i64::MAX as i128) as i64
}

fn fps_milli(rate: ffmpeg::Rational) -> i64 {
    if rate.numerator() <= 0 || rate.denominator() <= 0 {
        return 0;
    }
    let value = (rate.numerator() as i128) * 1000 / (rate.denominator() as i128);
    value.clamp(0, i64::MAX as i128) as i64
}

impl Video {
    pub fn open(path: &str) -> Option<Video> {
        init_ffmpeg();

        let input = format::input(&path).ok()?;
        let stream = input.streams().best(media::Type::Video)?;
        let stream_index = stream.index();
        let time_base = stream.time_base();
        let stream_duration = stream.duration();
        let fps = fps_milli(stream.avg_frame_rate());
        let has_audio = input.streams().best(media::Type::Audio).is_some();
        let parameters = stream.parameters();

        let context = codec::context::Context::from_parameters(parameters).ok()?;
        let decoder = context.decoder().video().ok()?;
        let width = decoder.width();
        let height = decoder.height();
        if width == 0 || height == 0 {
            return None;
        }

        let duration_ms = if stream_duration > 0 {
            stream_ms(stream_duration, time_base)
        } else {
            let fallback = input.duration();
            if fallback > 0 {
                fallback / 1000
            } else {
                0
            }
        };

        let shared = Arc::new(Shared {
            core: Mutex::new(Core {
                queue: VecDeque::with_capacity(QUEUE_LIMIT),
                seek_target: 0,
                seek_gen: 0,
            }),
            cv: Condvar::new(),
            stop: AtomicBool::new(false),
            width: i64::from(width),
            height: i64::from(height),
            duration_ms,
            fps_milli: fps,
            has_audio,
        });

        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("media-video-decode".into())
            .spawn(move || {
                worker_loop(
                    worker_shared,
                    input,
                    decoder,
                    None,
                    stream_index,
                    width,
                    height,
                )
            })
            .ok()?;

        Some(Video {
            shared,
            worker: Some(worker),
        })
    }

    pub fn info(&self, lane: i32) -> i64 {
        match lane {
            0 => self.shared.width,
            1 => self.shared.height,
            2 => self.shared.duration_ms,
            3 => self.shared.fps_milli,
            4 => i64::from(self.shared.has_audio),
            _ => -1,
        }
    }

    pub fn try_next(&mut self) -> Option<Vec<u8>> {
        let mut core = self.shared.core.lock().unwrap();
        let frame = core.queue.pop_front();
        if frame.is_some() {
            drop(core);
            self.shared.cv.notify_all();
        }
        frame
    }

    pub fn seek(&mut self, ms: i64) -> i64 {
        let target = ms.clamp(0, self.shared.duration_ms.max(0));
        {
            let mut core = self.shared.core.lock().unwrap();
            core.queue.clear();
            core.seek_target = target;
            core.seek_gen = core.seek_gen.wrapping_add(1);
        }
        self.shared.cv.notify_all();
        0
    }

    pub fn pending(&self) -> i64 {
        self.shared.core.lock().unwrap().queue.len() as i64
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.cv.notify_all();
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

fn push_frame(shared: &Shared, packed: Vec<u8>, gen: u64) -> bool {
    let mut core = shared.core.lock().unwrap();
    while core.queue.len() >= QUEUE_LIMIT {
        if shared.stop.load(Ordering::SeqCst) || core.seek_gen != gen {
            return false;
        }
        core = shared.cv.wait(core).unwrap();
    }
    if shared.stop.load(Ordering::SeqCst) || core.seek_gen != gen {
        return false;
    }
    core.queue.push_back(packed);
    shared.cv.notify_all();
    true
}

fn ensure_scaler(
    scaler: &mut Option<scaling::Context>,
    decoded: &frame::Video,
    width: u32,
    height: u32,
) {
    let format = decoded.format();
    let source_width = decoded.width();
    let source_height = decoded.height();
    if format == Pixel::None || source_width == 0 || source_height == 0 {
        *scaler = None;
        return;
    }
    if let Some(context) = scaler.as_ref() {
        if context.input().format == format
            && context.input().width == source_width
            && context.input().height == source_height
        {
            return;
        }
    }
    *scaler = scaling::Context::get(
        format,
        source_width,
        source_height,
        Pixel::RGBA,
        width,
        height,
        scaling::Flags::BILINEAR,
    )
    .ok();
}

fn drain_frames(
    shared: &Shared,
    decoder: &mut codec::decoder::Video,
    scaler: &mut Option<scaling::Context>,
    decoded: &mut frame::Video,
    rgba: &mut frame::Video,
    pixels: &mut Vec<u8>,
    width: u32,
    height: u32,
    gen: u64,
) -> bool {
    loop {
        if decoder.receive_frame(decoded).is_err() {
            return true;
        }
        ensure_scaler(scaler, decoded, width, height);
        let Some(context) = scaler.as_mut() else {
            continue;
        };
        if context.run(decoded, rgba).is_err() {
            *scaler = None;
            continue;
        }

        let row = width as usize * 4;
        let stride = rgba.stride(0);
        if stride < row {
            continue;
        }
        let data = rgba.data(0);
        if data.len() < stride * height as usize {
            continue;
        }
        pixels.clear();
        pixels.reserve(row * height as usize);
        for line in 0..height as usize {
            let start = line * stride;
            pixels.extend_from_slice(&data[start..start + row]);
        }

        let packed = rnxr::pack(width, height, pixels);
        if !push_frame(shared, packed, gen) {
            return false;
        }
    }
}

fn worker_loop(
    shared: Arc<Shared>,
    mut input: format::context::Input,
    mut decoder: codec::decoder::Video,
    mut scaler: Option<scaling::Context>,
    stream_index: usize,
    width: u32,
    height: u32,
) {
    let mut applied = 0u64;
    let mut eof = false;
    let mut decoded = frame::Video::empty();
    let mut rgba = frame::Video::empty();
    let mut pixels: Vec<u8> = Vec::new();

    loop {
        if shared.stop.load(Ordering::SeqCst) {
            break;
        }

        let request = {
            let mut core = shared.core.lock().unwrap();
            if core.seek_gen > applied {
                applied = core.seek_gen;
                core.queue.clear();
                Some(core.seek_target)
            } else {
                None
            }
        };

        if let Some(ms) = request {
            let ts = ms.saturating_mul(1000);
            let _ = input.clear_eof();
            let _ = input.seek(ts, ..ts);
            decoder.flush();
            eof = false;
            shared.cv.notify_all();
            continue;
        }

        if eof {
            shared.cv.notify_all();
            let mut core = shared.core.lock().unwrap();
            while !shared.stop.load(Ordering::SeqCst) && core.seek_gen == applied {
                core = shared.cv.wait(core).unwrap();
            }
            continue;
        }

        let mut packet = ffmpeg::Packet::empty();
        match packet.read(&mut input) {
            Ok(()) => {
                if packet.stream() != stream_index {
                    continue;
                }
                if decoder.send_packet(&packet).is_err() {
                    continue;
                }
                if !drain_frames(
                    &shared,
                    &mut decoder,
                    &mut scaler,
                    &mut decoded,
                    &mut rgba,
                    &mut pixels,
                    width,
                    height,
                    applied,
                ) {
                    continue;
                }
            }
            Err(_) => {
                let _ = decoder.send_eof();
                let _ = drain_frames(
                    &shared,
                    &mut decoder,
                    &mut scaler,
                    &mut decoded,
                    &mut rgba,
                    &mut pixels,
                    width,
                    height,
                    applied,
                );
                eof = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn decodes_first_frame_when_provided() {
        let Ok(path) = env::var("RNX_TEST_VIDEO") else {
            println!("skip: set RNX_TEST_VIDEO to a video file");
            return;
        };

        let mut video = Video::open(&path).expect("open video");
        assert!(video.info(0) > 0, "width {}", video.info(0));
        assert!(video.info(1) > 0, "height {}", video.info(1));

        let start = Instant::now();
        let mut packed = None;
        while start.elapsed() < Duration::from_secs(5) {
            if let Some(next) = video.try_next() {
                packed = Some(next);
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        let bytes = packed.expect("no frame within five seconds");
        let (width, height, length) = rnxr::parse_header(&bytes).expect("parse RNXR header");
        assert_eq!(length, width as usize * height as usize * 4, "pixel length");
        assert_eq!(bytes.len(), rnxr::HEADER + length);

        assert_eq!(video.seek(0), 0);
        assert_eq!(video.pending(), 0);
        assert!(video.info(3) >= 0);
        assert!(video.info(4) == 0 || video.info(4) == 1);
    }
}
