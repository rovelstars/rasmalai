//! Audio decode and playback.
//!
//! A decoder thread reads the file with FFmpeg and fills a bounded ring of
//! interleaved stereo f32 samples. A sink thread drains that ring into ALSA,
//! or into a pacing null sink when no device is available. Both threads share
//! one mutex and one condition variable, and both are joined by `Drop`.

use std::collections::VecDeque;
use std::env;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex, Once};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use alsa::pcm::{Access, Format as AlsaFormat, HwParams, PCM};
use alsa::{Direction, ValueOr};
use ffmpeg::format::{sample::Type as SampleType, Sample};
use ffmpeg::software::resampling;
use ffmpeg::{codec, format, frame, media, ChannelLayout};
use ffmpeg_next as ffmpeg;

const STATE_STOPPED: i64 = 0;
const STATE_PLAYING: i64 = 1;
const STATE_PAUSED: i64 = 2;
const STATE_ENDED: i64 = 3;

const TARGET_RATE: u32 = 48_000;
const CHUNK_FRAMES: usize = 1024;

const SAMPLE_FORMAT: Sample = Sample::F32(SampleType::Packed);

static FFMPEG_INIT: Once = Once::new();

fn init_ffmpeg() {
    FFMPEG_INIT.call_once(|| {
        let _ = ffmpeg::init();
    });
}

struct Core {
    samples: VecDeque<f32>,
    capacity: usize,
    state: i64,
    position_frames: u64,
    seek_target: i64,
    seek_gen: u64,
    decoder_done: bool,
}

struct Shared {
    core: Mutex<Core>,
    cv: Condvar,
    stop: AtomicBool,
    volume: AtomicI32,
    rate: u32,
    channels: u16,
    duration_ms: i64,
}

enum Sink {
    Alsa(PCM),
    Null { realtime: bool },
}

pub struct Audio {
    shared: Arc<Shared>,
    decoder: Option<JoinHandle<()>>,
    sink: Option<JoinHandle<()>>,
}

fn stream_ms(duration: i64, tb: ffmpeg::Rational) -> i64 {
    if duration <= 0 || tb.denominator() <= 0 {
        return 0;
    }
    let value = (duration as i128) * (tb.numerator() as i128) * 1000 / (tb.denominator() as i128);
    value.clamp(0, i64::MAX as i128) as i64
}

fn open_alsa(rate: u32, channels: u16) -> Option<(PCM, u32, u16)> {
    let pcm = PCM::new("default", Direction::Playback, false).ok()?;
    let (actual_rate, actual_channels) = {
        let hwp = HwParams::any(&pcm).ok()?;
        hwp.set_access(Access::RWInterleaved).ok()?;
        hwp.set_format(AlsaFormat::float()).ok()?;
        hwp.set_channels(u32::from(channels)).ok()?;
        let actual_rate = hwp.set_rate_near(rate, ValueOr::Nearest).ok()?;
        let _ = hwp.set_period_size_near(CHUNK_FRAMES as alsa::pcm::Frames, ValueOr::Nearest);
        pcm.hw_params(&hwp).ok()?;
        let actual_channels = hwp.get_channels().ok()?;
        if actual_channels > u32::from(u16::MAX) {
            return None;
        }
        (actual_rate, actual_channels as u16)
    };
    Some((pcm, actual_rate, actual_channels))
}

fn native_layout(layout: ChannelLayout, channels: u16) -> ChannelLayout {
    if layout.is_empty() || layout.bits() == 0 || layout.channels() <= 0 {
        ChannelLayout::default(i32::from(channels.max(1)))
    } else {
        layout
    }
}

fn make_resampler(
    decoder: &codec::decoder::Audio,
    rate: u32,
    layout: ChannelLayout,
) -> Option<resampling::Context> {
    if decoder.format() == Sample::None || decoder.rate() == 0 {
        return None;
    }
    let source = native_layout(decoder.channel_layout(), decoder.channels());
    resampling::Context::get(
        decoder.format(),
        source,
        decoder.rate(),
        SAMPLE_FORMAT,
        layout,
        rate,
    )
    .ok()
}

impl Audio {
    pub fn open(path: &str) -> Option<Audio> {
        init_ffmpeg();

        let input = format::input(&path).ok()?;
        let stream = input.streams().best(media::Type::Audio)?;
        let stream_index = stream.index();
        let duration_ms = stream_ms(stream.duration(), stream.time_base());
        let parameters = stream.parameters();
        let context = codec::context::Context::from_parameters(parameters).ok()?;
        let decoder = context.decoder().audio().ok()?;

        let source_rate = decoder.rate();
        if source_rate == 0 {
            return None;
        }

        let mut rate = TARGET_RATE;
        let mut layout = ChannelLayout::STEREO;
        let mut resampler = match make_resampler(&decoder, rate, layout) {
            Some(value) => value,
            None => {
                rate = source_rate;
                match make_resampler(&decoder, rate, layout) {
                    Some(value) => value,
                    None => {
                        layout = native_layout(decoder.channel_layout(), decoder.channels());
                        make_resampler(&decoder, rate, layout)?
                    }
                }
            }
        };
        let channels = layout.channels().clamp(1, i32::from(u16::MAX)) as u16;

        let mode = env::var("RNX_AUDIO").unwrap_or_default();
        let use_device = mode != "null" && mode != "fast";
        let mut device = if use_device {
            open_alsa(rate, channels)
        } else {
            None
        };

        if let Some((_, actual_rate, actual_channels)) = device.as_ref() {
            if *actual_channels != channels {
                device = None;
            } else if *actual_rate != rate {
                match make_resampler(&decoder, *actual_rate, layout) {
                    Some(next) => {
                        resampler = next;
                        rate = *actual_rate;
                    }
                    None => device = None,
                }
            }
        }

        let sink = match device {
            Some((pcm, _, _)) => Sink::Alsa(pcm),
            None => Sink::Null {
                realtime: mode != "fast",
            },
        };

        let duration_ms = if duration_ms > 0 {
            duration_ms
        } else {
            let fallback = input.duration();
            if fallback > 0 {
                fallback / 1000
            } else {
                0
            }
        };

        let capacity = (rate as usize)
            .saturating_mul(channels as usize)
            .saturating_mul(2)
            .max(channels as usize);

        let shared = Arc::new(Shared {
            core: Mutex::new(Core {
                samples: VecDeque::with_capacity(capacity),
                capacity,
                state: STATE_PAUSED,
                position_frames: 0,
                seek_target: 0,
                seek_gen: 0,
                decoder_done: false,
            }),
            cv: Condvar::new(),
            stop: AtomicBool::new(false),
            volume: AtomicI32::new(100),
            rate,
            channels,
            duration_ms,
        });

        let sink_shared = Arc::clone(&shared);
        let sink_thread = thread::Builder::new()
            .name("media-audio-sink".into())
            .spawn(move || sink_thread(sink_shared, sink))
            .ok()?;

        let decode_shared = Arc::clone(&shared);
        let decoder_thread = thread::Builder::new()
            .name("media-audio-decode".into())
            .spawn(move || {
                decode_thread(
                    decode_shared,
                    input,
                    decoder,
                    resampler,
                    stream_index,
                    rate,
                    channels,
                )
            });
        let decoder_thread = match decoder_thread {
            Ok(handle) => handle,
            Err(_) => {
                shared.stop.store(true, Ordering::SeqCst);
                shared.cv.notify_all();
                let _ = sink_thread.join();
                return None;
            }
        };

        Some(Audio {
            shared,
            decoder: Some(decoder_thread),
            sink: Some(sink_thread),
        })
    }

    pub fn play(&self) -> i64 {
        {
            let mut core = self.shared.core.lock().unwrap();
            if core.state == STATE_ENDED {
                core.samples.clear();
                core.position_frames = 0;
                core.seek_target = 0;
                core.seek_gen = core.seek_gen.wrapping_add(1);
                core.decoder_done = false;
            }
            core.state = STATE_PLAYING;
        }
        self.shared.cv.notify_all();
        0
    }

    pub fn pause(&self) -> i64 {
        {
            let mut core = self.shared.core.lock().unwrap();
            if core.state == STATE_PLAYING {
                core.state = STATE_PAUSED;
            }
        }
        self.shared.cv.notify_all();
        0
    }

    pub fn stop(&self) -> i64 {
        {
            let mut core = self.shared.core.lock().unwrap();
            core.state = STATE_STOPPED;
            core.samples.clear();
            core.position_frames = 0;
            core.seek_target = 0;
            core.seek_gen = core.seek_gen.wrapping_add(1);
            core.decoder_done = false;
        }
        self.shared.cv.notify_all();
        0
    }

    pub fn seek(&self, ms: i64) -> i64 {
        let target = ms.clamp(0, self.shared.duration_ms.max(0));
        {
            let mut core = self.shared.core.lock().unwrap();
            core.samples.clear();
            core.seek_target = target;
            core.seek_gen = core.seek_gen.wrapping_add(1);
            core.decoder_done = false;
            core.position_frames = ((target as u128) * (self.shared.rate as u128) / 1000) as u64;
            if core.state == STATE_ENDED {
                core.state = STATE_PAUSED;
            }
        }
        self.shared.cv.notify_all();
        target
    }

    pub fn position(&self) -> i64 {
        let core = self.shared.core.lock().unwrap();
        let rate = u128::from(self.shared.rate.max(1));
        ((u128::from(core.position_frames) * 1000) / rate) as i64
    }

    pub fn duration(&self) -> i64 {
        self.shared.duration_ms
    }

    pub fn channels(&self) -> i64 {
        i64::from(self.shared.channels)
    }

    pub fn rate(&self) -> i64 {
        i64::from(self.shared.rate)
    }

    pub fn volume(&self, percent: i32) -> i64 {
        let clamped = percent.clamp(0, 200);
        self.shared.volume.store(clamped, Ordering::Relaxed);
        0
    }

    pub fn state(&self) -> i64 {
        self.shared.core.lock().unwrap().state
    }

    pub fn pending(&self) -> i64 {
        self.shared.core.lock().unwrap().samples.len() as i64
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.cv.notify_all();
        if let Some(handle) = self.decoder.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.sink.take() {
            let _ = handle.join();
        }
    }
}

fn push_samples(shared: &Shared, data: &[f32], gen: u64) -> bool {
    let mut offset = 0;
    let mut core = shared.core.lock().unwrap();
    while offset < data.len() {
        if shared.stop.load(Ordering::SeqCst) || core.seek_gen != gen {
            return false;
        }
        let room = core.capacity.saturating_sub(core.samples.len());
        if room == 0 {
            core = shared.cv.wait(core).unwrap();
            continue;
        }
        let take = room.min(data.len() - offset);
        core.samples
            .extend(data[offset..offset + take].iter().copied());
        offset += take;
        shared.cv.notify_all();
    }
    true
}

fn decode_thread(
    shared: Arc<Shared>,
    mut input: format::context::Input,
    mut decoder: codec::decoder::Audio,
    mut resampler: resampling::Context,
    stream_index: usize,
    rate: u32,
    channels: u16,
) {
    let mut applied = 0u64;
    let mut eof = false;
    let mut converted = frame::Audio::empty();
    let mut capacity = 0usize;
    let layout = ChannelLayout::default(i32::from(channels));

    loop {
        if shared.stop.load(Ordering::SeqCst) {
            break;
        }

        let request = {
            let mut core = shared.core.lock().unwrap();
            if core.seek_gen > applied {
                applied = core.seek_gen;
                core.samples.clear();
                core.decoder_done = false;
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
            match make_resampler(&decoder, rate, layout) {
                Some(next) => {
                    resampler = next;
                    converted = frame::Audio::empty();
                    capacity = 0;
                }
                None => break,
            }
            eof = false;
            shared.cv.notify_all();
            continue;
        }

        if eof {
            {
                let mut core = shared.core.lock().unwrap();
                core.decoder_done = true;
            }
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
                    &mut resampler,
                    &mut converted,
                    &mut capacity,
                    layout,
                    channels,
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
                    &mut resampler,
                    &mut converted,
                    &mut capacity,
                    layout,
                    channels,
                    applied,
                );
                flush_resampler(
                    &shared,
                    &mut resampler,
                    &mut converted,
                    &mut capacity,
                    layout,
                    channels,
                    applied,
                );
                eof = true;
            }
        }
    }
}

fn drain_frames(
    shared: &Shared,
    decoder: &mut codec::decoder::Audio,
    resampler: &mut resampling::Context,
    converted: &mut frame::Audio,
    capacity: &mut usize,
    output_layout: ChannelLayout,
    channels: u16,
    gen: u64,
) -> bool {
    let mut decoded = frame::Audio::empty();

    loop {
        if decoder.receive_frame(&mut decoded).is_err() {
            return true;
        }
        if decoded.format() == Sample::None || decoded.samples() == 0 {
            continue;
        }
        let layout = native_layout(decoded.channel_layout(), decoded.channels());
        if layout != decoded.channel_layout() {
            decoded.set_channel_layout(layout);
        }
        if layout != resampler.input().channel_layout {
            let output = *resampler.output();
            let rate = if decoded.rate() > 0 {
                decoded.rate()
            } else {
                resampler.input().rate
            };
            match resampling::Context::get(
                decoded.format(),
                layout,
                rate,
                output.format,
                output.channel_layout,
                output.rate,
            ) {
                Ok(next) => {
                    *resampler = next;
                    *converted = frame::Audio::empty();
                    *capacity = 0;
                }
                Err(_) => continue,
            }
        }
        let source_rate = resampler.input().rate.max(1);
        let output_rate = resampler.output().rate.max(1);
        let needed = ((decoded.samples() as u64 * u64::from(output_rate) + u64::from(source_rate)
            - 1)
            / u64::from(source_rate)) as usize
            + 64;
        if needed > *capacity {
            *converted = frame::Audio::new(SAMPLE_FORMAT, needed, output_layout);
            *capacity = needed;
        } else {
            converted.set_samples(needed);
        }
        if resampler.run(&decoded, converted).is_err() {
            continue;
        }
        if !emit(shared, converted, channels, gen) {
            return false;
        }
    }
}

fn flush_resampler(
    shared: &Shared,
    resampler: &mut resampling::Context,
    converted: &mut frame::Audio,
    capacity: &mut usize,
    output_layout: ChannelLayout,
    channels: u16,
    gen: u64,
) {
    for _ in 0..8 {
        if *capacity == 0 {
            *converted = frame::Audio::new(SAMPLE_FORMAT, CHUNK_FRAMES * 4, output_layout);
            *capacity = CHUNK_FRAMES * 4;
        } else {
            converted.set_samples(*capacity);
        }
        match resampler.flush(converted) {
            Ok(_) => {}
            Err(_) => return,
        }
        if converted.samples() == 0 {
            return;
        }
        if !emit(shared, converted, channels, gen) {
            return;
        }
    }
}

fn emit(shared: &Shared, converted: &frame::Audio, channels: u16, gen: u64) -> bool {
    if converted.format() != SAMPLE_FORMAT
        || converted.samples() == 0
        || converted.channels() != channels
    {
        return true;
    }
    if converted.planes() == 0 {
        return true;
    }
    let data = converted.plane::<f32>(0);
    push_samples(shared, data, gen)
}

fn sink_thread(shared: Arc<Shared>, sink: Sink) {
    let rate = shared.rate.max(1) as usize;
    let channels = shared.channels.max(1) as usize;
    let mut scratch = vec![0.0f32; CHUNK_FRAMES * channels];
    let io = match &sink {
        Sink::Alsa(pcm) => pcm.io_f32().ok(),
        Sink::Null { .. } => None,
    };
    let mut deadline: Option<Instant> = None;

    loop {
        let count = {
            let mut core = shared.core.lock().unwrap();
            loop {
                if shared.stop.load(Ordering::SeqCst) {
                    return;
                }
                if core.state != STATE_PLAYING {
                    deadline = None;
                    core = shared.cv.wait(core).unwrap();
                    continue;
                }
                if core.samples.is_empty() {
                    if core.decoder_done {
                        core.state = STATE_ENDED;
                        deadline = None;
                        shared.cv.notify_all();
                    }
                    core = shared.cv.wait(core).unwrap();
                    continue;
                }
                break;
            }
            let count = core.samples.len().min(scratch.len());
            for slot in scratch[..count].iter_mut() {
                *slot = core.samples.pop_front().unwrap();
            }
            shared.cv.notify_all();
            count
        };

        let volume = shared.volume.load(Ordering::Relaxed) as f32 / 100.0;
        if (volume - 1.0).abs() > f32::EPSILON {
            for sample in &mut scratch[..count] {
                *sample *= volume;
            }
        }

        let frames = count / channels;
        if frames == 0 {
            continue;
        }

        let consumed = match &sink {
            Sink::Alsa(pcm) => {
                let Some(io) = io.as_ref() else {
                    return;
                };
                let mut written = 0usize;
                while written < count {
                    match io.writei(&scratch[written..count]) {
                        Ok(0) => break,
                        Ok(progress) => written += progress * channels,
                        Err(err) => {
                            if pcm.try_recover(err, false).is_err() || pcm.prepare().is_err() {
                                return;
                            }
                        }
                    }
                }
                written / channels
            }
            Sink::Null { realtime: true } => {
                let now = Instant::now();
                let step = Duration::from_secs_f64(frames as f64 / rate as f64);
                let target = deadline.unwrap_or(now) + step;
                if target > now {
                    thread::sleep(target - now);
                }
                deadline = Some(target);
                frames
            }
            Sink::Null { realtime: false } => {
                thread::yield_now();
                frames
            }
        };

        if consumed > 0 {
            let mut core = shared.core.lock().unwrap();
            core.position_frames += consumed as u64;
            drop(core);
            shared.cv.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::Video;
    use std::fs;
    use std::path::Path;
    use std::sync::Mutex as StdMutex;

    static ENV_LOCK: StdMutex<()> = StdMutex::new(());

    fn write_wav(path: &Path, rate: u32, channels: u16, frames: u32) {
        let data_len = frames * u32::from(channels) * 2;
        let mut bytes = Vec::with_capacity(44 + data_len as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        bytes.extend_from_slice(&(channels * 2).to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for n in 0..frames {
            let t = n as f32 / rate as f32;
            let value = (t * 440.0 * std::f32::consts::TAU).sin() * 0.25;
            let sample = (value * f32::from(i16::MAX)) as i16;
            for _ in 0..channels {
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
        }
        fs::write(path, &bytes).expect("write wav");
    }

    #[test]
    fn wav_playback_and_seek() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let dir = env::temp_dir().join(format!("media-audio-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("tone.wav");
        write_wav(&path, 44_100, 2, 22_050);

        let previous = env::var("RNX_AUDIO").ok();
        env::set_var("RNX_AUDIO", "fast");

        let audio = Audio::open(path.to_str().expect("path")).expect("open wav");
        assert_eq!(audio.channels(), 2);
        assert_eq!(audio.rate(), 48_000);
        let duration = audio.duration();
        assert!((duration - 500).abs() <= 20, "duration {duration}");
        assert_eq!(audio.position(), 0);

        assert_eq!(audio.play(), 0);
        let start = Instant::now();
        while audio.state() != STATE_ENDED {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "playback stalled at state {}",
                audio.state()
            );
            thread::sleep(Duration::from_millis(10));
        }
        let position = audio.position();
        assert!(
            (position - duration).abs() <= 60,
            "position {position} duration {duration}"
        );

        let seeked = audio.seek(100);
        assert_eq!(seeked, 100);
        let after = audio.position();
        assert!(
            (0..=duration).contains(&after),
            "seek position {after} out of range"
        );

        let clamped = audio.seek(duration + 10_000);
        assert_eq!(clamped, duration);
        assert!((0..=duration).contains(&audio.position()));

        drop(audio);
        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
        match previous {
            Some(value) => env::set_var("RNX_AUDIO", value),
            None => env::remove_var("RNX_AUDIO"),
        }
    }

    #[test]
    fn missing_file_opens_none() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let path = env::temp_dir().join(format!("media-missing-{}.bin", std::process::id()));
        fs::remove_file(&path).ok();
        let name = path.to_str().expect("path");
        assert!(Audio::open(name).is_none());
        assert!(Video::open(name).is_none());
    }
}
