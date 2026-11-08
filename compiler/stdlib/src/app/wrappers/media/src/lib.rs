//! Media C ABI: audio playback, video frames, camera frames, and a file
//! picker.
//!
//! Every handle owns its worker threads and bounded buffers. Closing a handle
//! stops and joins those workers. A closed or foreign handle returns a
//! negative code instead of touching freed memory.

#[cfg(not(target_os = "linux"))]
compile_error!(
    "the Rasmalai media wrapper supports Linux only (ALSA, V4L2, XDG portal); see requirements_UI/72-platform-support.md"
);

mod audio;
mod camera;
mod frame;
mod picker;
mod video;

use std::ffi::c_void;
use std::ptr;
use std::slice;

pub use frame::{HEADER, MAGIC};

enum Media {
    Audio(audio::Audio),
    Video(video::Video),
    Camera(camera::Camera),
}

unsafe fn bytes<'a>(ptr: *const u8, len: i32) -> Option<&'a [u8]> {
    if ptr.is_null() || len < 0 {
        return None;
    }
    Some(slice::from_raw_parts(ptr, len as usize))
}

unsafe fn text<'a>(ptr: *const u8, len: i32) -> Option<&'a str> {
    std::str::from_utf8(bytes(ptr, len)?).ok()
}

fn copy_out(out: *mut u8, cap: i32, data: &[u8]) -> i64 {
    if out.is_null() || cap < 0 {
        return -1;
    }
    if data.len() > cap as usize {
        return -4;
    }
    unsafe {
        ptr::copy_nonoverlapping(data.as_ptr(), out, data.len());
    }
    data.len() as i64
}

unsafe fn media<'a>(h: *mut c_void) -> Option<&'a mut Media> {
    if h.is_null() {
        return None;
    }
    (h as *mut Media).as_mut()
}

unsafe fn audio<'a>(h: *mut c_void) -> Option<&'a mut audio::Audio> {
    match media(h)? {
        Media::Audio(a) => Some(a),
        _ => None,
    }
}

unsafe fn video<'a>(h: *mut c_void) -> Option<&'a mut video::Video> {
    match media(h)? {
        Media::Video(v) => Some(v),
        _ => None,
    }
}

unsafe fn camera<'a>(h: *mut c_void) -> Option<&'a mut camera::Camera> {
    match media(h)? {
        Media::Camera(c) => Some(c),
        _ => None,
    }
}

fn into_handle(m: Media) -> *mut c_void {
    Box::into_raw(Box::new(m)) as *mut c_void
}

#[no_mangle]
pub extern "C" fn media_abi_version() -> i64 {
    1
}

#[no_mangle]
pub extern "C" fn media_handle_address(h: *mut c_void) -> i64 {
    h as usize as i64
}

#[no_mangle]
pub extern "C" fn media_audio_open(path: *const u8, len: i32) -> *mut c_void {
    let Some(path) = (unsafe { text(path, len) }) else {
        return ptr::null_mut();
    };
    match audio::Audio::open(path) {
        Some(a) => into_handle(Media::Audio(a)),
        None => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn media_audio_close(h: *mut c_void) {
    if h.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(h as *mut Media));
    }
}

#[no_mangle]
pub extern "C" fn media_audio_play(h: *mut c_void) -> i64 {
    unsafe { audio(h) }.map_or(-1, |a| a.play())
}

#[no_mangle]
pub extern "C" fn media_audio_pause(h: *mut c_void) -> i64 {
    unsafe { audio(h) }.map_or(-1, |a| a.pause())
}

#[no_mangle]
pub extern "C" fn media_audio_stop(h: *mut c_void) -> i64 {
    unsafe { audio(h) }.map_or(-1, |a| a.stop())
}

#[no_mangle]
pub extern "C" fn media_audio_seek(h: *mut c_void, ms: i64) -> i64 {
    unsafe { audio(h) }.map_or(-1, |a| a.seek(ms))
}

#[no_mangle]
pub extern "C" fn media_audio_position(h: *mut c_void) -> i64 {
    unsafe { audio(h) }.map_or(-1, |a| a.position())
}

#[no_mangle]
pub extern "C" fn media_audio_duration(h: *mut c_void) -> i64 {
    unsafe { audio(h) }.map_or(-1, |a| a.duration())
}

#[no_mangle]
pub extern "C" fn media_audio_channels(h: *mut c_void) -> i64 {
    unsafe { audio(h) }.map_or(-1, |a| a.channels())
}

#[no_mangle]
pub extern "C" fn media_audio_rate(h: *mut c_void) -> i64 {
    unsafe { audio(h) }.map_or(-1, |a| a.rate())
}

#[no_mangle]
pub extern "C" fn media_audio_volume(h: *mut c_void, percent: i32) -> i64 {
    unsafe { audio(h) }.map_or(-1, |a| a.volume(percent))
}

#[no_mangle]
pub extern "C" fn media_audio_state(h: *mut c_void) -> i64 {
    unsafe { audio(h) }.map_or(-1, |a| a.state())
}

#[no_mangle]
pub extern "C" fn media_audio_pending(h: *mut c_void) -> i64 {
    unsafe { audio(h) }.map_or(-1, |a| a.pending())
}

#[no_mangle]
pub extern "C" fn media_video_open(path: *const u8, len: i32) -> *mut c_void {
    let Some(path) = (unsafe { text(path, len) }) else {
        return ptr::null_mut();
    };
    match video::Video::open(path) {
        Some(v) => into_handle(Media::Video(v)),
        None => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn media_video_close(h: *mut c_void) {
    if h.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(h as *mut Media));
    }
}

#[no_mangle]
pub extern "C" fn media_video_info(h: *mut c_void, lane: i32) -> i64 {
    unsafe { video(h) }.map_or(-1, |v| v.info(lane))
}

#[no_mangle]
pub extern "C" fn media_video_next(h: *mut c_void, out: *mut u8, cap: i32) -> i64 {
    let Some(v) = (unsafe { video(h) }) else {
        return -1;
    };
    match v.try_next() {
        Some(bytes) => copy_out(out, cap, &bytes),
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn media_video_seek(h: *mut c_void, ms: i64) -> i64 {
    unsafe { video(h) }.map_or(-1, |v| v.seek(ms))
}

#[no_mangle]
pub extern "C" fn media_video_pending(h: *mut c_void) -> i64 {
    unsafe { video(h) }.map_or(-1, |v| v.pending())
}

#[no_mangle]
pub extern "C" fn media_camera_open(
    device: *const u8,
    len: i32,
    w: i32,
    h: i32,
) -> *mut c_void {
    let Some(device) = (unsafe { text(device, len) }) else {
        return ptr::null_mut();
    };
    match camera::Camera::open(device, w, h) {
        Some(c) => into_handle(Media::Camera(c)),
        None => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn media_camera_close(h: *mut c_void) {
    if h.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(h as *mut Media));
    }
}

#[no_mangle]
pub extern "C" fn media_camera_info(h: *mut c_void, lane: i32) -> i64 {
    unsafe { camera(h) }.map_or(-1, |c| c.info(lane))
}

#[no_mangle]
pub extern "C" fn media_camera_next(h: *mut c_void, out: *mut u8, cap: i32) -> i64 {
    let Some(c) = (unsafe { camera(h) }) else {
        return -1;
    };
    match c.try_next() {
        Some(bytes) => copy_out(out, cap, &bytes),
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn media_camera_pending(h: *mut c_void) -> i64 {
    unsafe { camera(h) }.map_or(-1, |c| c.pending())
}

#[no_mangle]
pub extern "C" fn media_pick_file(
    title: *const u8,
    len: i32,
    filters: *const u8,
    filters_len: i32,
    out: *mut u8,
    cap: i32,
) -> i64 {
    let Some(title) = (unsafe { text(title, len) }) else {
        return -1;
    };
    let filters = unsafe { text(filters, filters_len) }.unwrap_or("");
    match picker::pick_file(title, filters) {
        Some(path) => copy_out(out, cap, path.as_bytes()),
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn media_pick_files(
    title: *const u8,
    len: i32,
    filters: *const u8,
    filters_len: i32,
    out: *mut u8,
    cap: i32,
) -> i64 {
    let Some(title) = (unsafe { text(title, len) }) else {
        return -1;
    };
    let filters = unsafe { text(filters, filters_len) }.unwrap_or("");
    let paths = picker::pick_files(title, filters);
    if paths.is_empty() {
        return 0;
    }
    let joined = paths.join("\n");
    copy_out(out, cap, joined.as_bytes())
}

/// Kept for callers that need a stable string ABI test.
#[no_mangle]
pub extern "C" fn media_echo(ptr_: *const u8, len: i32, out: *mut u8, cap: i32) -> i64 {
    let Some(s) = (unsafe { text(ptr_, len) }) else {
        return -1;
    };
    copy_out(out, cap, s.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_frame_round_trip() {
        let pixels = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let packed = frame::pack(2, 1, &pixels);
        assert_eq!(packed.len(), HEADER + 8);
        let (w, h, len) = frame::parse_header(&packed).unwrap();
        assert_eq!((w, h, len), (2, 1, 8));
        assert_eq!(&packed[HEADER..], &pixels[..]);
    }

    #[test]
    fn foreign_handles_fail_closed() {
        let mut value = 0u8;
        let bad = (&mut value) as *mut u8 as *mut c_void;
        assert_eq!(media_audio_state(bad), -1);
        assert_eq!(media_video_info(bad, 0), -1);
        assert_eq!(media_camera_info(bad, 0), -1);
    }
}

