//! Minimal surface socket transport.
//!
//! One message per window. A commit carries the display list, the optional
//! frame packet, and the damage rectangles; the compositor answers with an ack
//! carrying the same window id and a status. The encoding is little-endian.
//!
//! Hello  (20 bytes): b"EPIC", version, width, height, window id.
//! Frame  (32 bytes): b"EPIC", kind=1, window id, width, height,
//!                    list length, packet length, damage count, then
//!                    list bytes, packet bytes, and 4 x i32 per damage rect.
//! Ack    (16 bytes): b"EPAK", kind=2, window id, status.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};

const MAGIC: &[u8; 4] = b"EPIC";
const ACK_MAGIC: &[u8; 4] = b"EPAK";
const VERSION: u32 = 1;
const KIND_FRAME: u32 = 1;
const KIND_ACK: u32 = 2;

pub struct SurfaceSocketHost {
    stream: UnixStream,
    id: u32,
    width: u32,
    height: u32,
}

pub struct Ack {
    pub id: u32,
    pub status: i32,
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn get_u32(data: &[u8], at: usize) -> Option<u32> {
    let end = at.checked_add(4)?;
    let bytes = data.get(at..end)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

pub fn encode_hello(id: u32, width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(20);
    out.extend_from_slice(MAGIC);
    put_u32(&mut out, VERSION);
    put_u32(&mut out, width);
    put_u32(&mut out, height);
    put_u32(&mut out, id);
    out
}

pub fn encode_frame(
    id: u32,
    width: u32,
    height: u32,
    list: &[u8],
    packet: &[u8],
    damage: &[(i32, i32, i32, i32)],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + list.len() + packet.len() + damage.len() * 16);
    out.extend_from_slice(MAGIC);
    put_u32(&mut out, KIND_FRAME);
    put_u32(&mut out, id);
    put_u32(&mut out, width);
    put_u32(&mut out, height);
    put_u32(&mut out, list.len() as u32);
    put_u32(&mut out, packet.len() as u32);
    put_u32(&mut out, damage.len() as u32);
    out.extend_from_slice(list);
    out.extend_from_slice(packet);
    for (x, y, w, h) in damage {
        put_i32(&mut out, *x);
        put_i32(&mut out, *y);
        put_i32(&mut out, *w);
        put_i32(&mut out, *h);
    }
    out
}

pub fn decode_ack(data: &[u8]) -> Option<Ack> {
    if data.len() < 16 || &data[0..4] != ACK_MAGIC {
        return None;
    }
    if get_u32(data, 4)? != KIND_ACK {
        return None;
    }
    Some(Ack {
        id: get_u32(data, 8)?,
        status: get_u32(data, 12)? as i32,
    })
}

fn read_exact(stream: &mut UnixStream, len: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

impl SurfaceSocketHost {
    pub fn connect(path: &str, id: u32, width: u32, height: u32) -> std::io::Result<Self> {
        let mut stream = UnixStream::connect(path)?;
        stream.write_all(&encode_hello(id, width, height))?;
        stream.flush()?;
        Ok(Self {
            stream,
            id,
            width,
            height,
        })
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    pub fn commit(
        &mut self,
        list: &[u8],
        packet: &[u8],
        damage: &[(i32, i32, i32, i32)],
    ) -> std::io::Result<i32> {
        let frame = encode_frame(
            self.id,
            self.width,
            self.height,
            list,
            packet,
            damage,
        );
        self.stream.write_all(&frame)?;
        self.stream.flush()?;
        let ack = read_exact(&mut self.stream, 16)?;
        match decode_ack(&ack) {
            Some(a) if a.id == self.id => Ok(a.status),
            Some(_) => Ok(-2),
            None => Ok(-3),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener as TestListener;

    fn temp_path(name: &str) -> String {
        let mut path = std::env::temp_dir();
        path.push(format!("rnx-surface-{}-{name}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn frame_and_window_id_round_trip() {
        let path = temp_path("round-trip");
        let listener = TestListener::bind(&path).expect("bind");
        let server = std::thread::spawn({
            let path = path.clone();
            move || {
                let (mut stream, _) = listener.accept().expect("accept");
                let hello = read_exact(&mut stream, 20).expect("hello");
                assert_eq!(&hello[0..4], MAGIC);
                assert_eq!(get_u32(&hello, 4), Some(VERSION));
                let id = get_u32(&hello, 16).expect("id");
                let header = read_exact(&mut stream, 32).expect("frame header");
                assert_eq!(&header[0..4], MAGIC);
                assert_eq!(get_u32(&header, 4), Some(KIND_FRAME));
                let frame_id = get_u32(&header, 8).expect("frame id");
                let list_len = get_u32(&header, 20).expect("list len") as usize;
                let packet_len = get_u32(&header, 24).expect("packet len") as usize;
                let damage_count = get_u32(&header, 28).expect("damage") as usize;
                let list = read_exact(&mut stream, list_len).expect("list");
                let packet = read_exact(&mut stream, packet_len).expect("packet");
                let damage = read_exact(&mut stream, damage_count * 16).expect("damage");
                let mut ack = Vec::new();
                ack.extend_from_slice(ACK_MAGIC);
                put_u32(&mut ack, KIND_ACK);
                put_u32(&mut ack, frame_id);
                put_u32(&mut ack, 0);
                stream.write_all(&ack).expect("ack");
                let _ = std::fs::remove_file(&path);
                (id, frame_id, list, packet, damage)
            }
        });

        let mut client = SurfaceSocketHost::connect(&path, 7, 320, 200).expect("connect");
        let status = client
            .commit(&[9, 8, 7], &[1, 2], &[(0, 0, 320, 200)])
            .expect("commit");
        assert_eq!(status, 0);
        let (hello_id, frame_id, list, packet, damage) = server.join().expect("server");
        assert_eq!(hello_id, 7, "hello carries the window id");
        assert_eq!(frame_id, 7, "frame carries the window id");
        assert_eq!(list, vec![9, 8, 7]);
        assert_eq!(packet, vec![1, 2]);
        assert_eq!(damage.len(), 16);
    }

    #[test]
    fn two_windows_commit_independently() {
        let path = temp_path("two-windows");
        let listener = TestListener::bind(&path).expect("bind");
        let server = std::thread::spawn({
            let path = path.clone();
            move || {
                let mut ids = Vec::new();
                for _ in 0..2 {
                    let (mut stream, _) = listener.accept().expect("accept");
                    let hello = read_exact(&mut stream, 20).expect("hello");
                    ids.push(get_u32(&hello, 16).expect("hello id"));
                    let header = read_exact(&mut stream, 32).expect("header");
                    let id = get_u32(&header, 8).expect("frame id");
                    ids.push(id);
                    let list_len = get_u32(&header, 20).expect("list len") as usize;
                    let packet_len = get_u32(&header, 24).expect("packet") as usize;
                    let damage = get_u32(&header, 28).expect("damage") as usize;
                    let _ = read_exact(&mut stream, list_len + packet_len + damage * 16);
                    let mut ack = Vec::new();
                    ack.extend_from_slice(ACK_MAGIC);
                    put_u32(&mut ack, KIND_ACK);
                    put_u32(&mut ack, id);
                    put_u32(&mut ack, 0);
                    stream.write_all(&ack).expect("ack");
                }
                let _ = std::fs::remove_file(&path);
                ids
            }
        });

        for id in [3u32, 9u32] {
            let mut client = SurfaceSocketHost::connect(&path, id, 100, 80).expect("connect");
            assert_eq!(client.commit(&[1], &[], &[]).expect("commit"), 0);
        }
        let ids = server.join().expect("server");
        assert_eq!(ids, vec![3, 3, 9, 9]);
    }

    #[test]
    fn rejects_bad_ack() {
        assert!(decode_ack(b"nope").is_none());
        let mut ack = Vec::new();
        ack.extend_from_slice(ACK_MAGIC);
        put_u32(&mut ack, KIND_ACK);
        put_u32(&mut ack, 5);
        put_u32(&mut ack, 0);
        assert_eq!(decode_ack(&ack).map(|a| a.id), Some(5));
    }
}

/// Development server used by the app test and probes. It accepts hello and
/// frame messages, records the last window id and payload sizes, and acks.
#[derive(Default)]
pub struct TestStats {
    pub frames: usize,
    pub last_id: u32,
    pub last_list: usize,
    pub last_packet: usize,
    pub last_damage: usize,
}

struct TestServer {
    path: String,
    stats: std::sync::Arc<std::sync::Mutex<TestStats>>,
}

static TEST_SERVERS: std::sync::OnceLock<std::sync::Mutex<Vec<TestServer>>> =
    std::sync::OnceLock::new();

fn test_servers() -> &'static std::sync::Mutex<Vec<TestServer>> {
    TEST_SERVERS.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

pub fn start_test_server(path: &str) -> std::io::Result<()> {
    let listener = UnixListener::bind(path)?;
    let stats = std::sync::Arc::new(std::sync::Mutex::new(TestStats::default()));
    let shared = std::sync::Arc::clone(&stats);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let Ok(hello) = read_exact(&mut stream, 20) else { continue };
            loop {
                let Ok(header) = read_exact(&mut stream, 32) else { break };
                let Some(id) = get_u32(&header, 8) else { break };
                let Some(list_len) = get_u32(&header, 20) else { break };
                let Some(packet_len) = get_u32(&header, 24) else { break };
                let Some(damage) = get_u32(&header, 28) else { break };
                if read_exact(
                    &mut stream,
                    list_len as usize + packet_len as usize + damage as usize * 16,
                )
                .is_err()
                {
                    break;
                }
                let _ = &hello;
                if let Ok(mut stats) = shared.lock() {
                    stats.frames += 1;
                    stats.last_id = id;
                    stats.last_list = list_len as usize;
                    stats.last_packet = packet_len as usize;
                    stats.last_damage = damage as usize;
                }
                let mut ack = Vec::new();
                ack.extend_from_slice(ACK_MAGIC);
                put_u32(&mut ack, KIND_ACK);
                put_u32(&mut ack, id);
                put_u32(&mut ack, 0);
                if stream.write_all(&ack).is_err() {
                    break;
                }
            }
        }
    });
    test_servers()
        .lock()
        .map_err(|_| std::io::Error::other("test server lock"))?
        .push(TestServer {
            path: path.to_string(),
            stats,
        });
    Ok(())
}

fn with_stats<T>(path: &str, f: impl FnOnce(&TestStats) -> T) -> Option<T> {
    let servers = test_servers().lock().ok()?;
    let server = servers.iter().find(|s| s.path == path)?;
    let stats = server.stats.lock().ok()?;
    Some(f(&stats))
}

pub fn test_lookup(path: *const u8, len: i32, f: impl FnOnce(&TestStats) -> i64) -> i64 {
    if path.is_null() || len < 0 {
        return -1;
    }
    let bytes = unsafe { std::slice::from_raw_parts(path, len as usize) };
    let Ok(path) = std::str::from_utf8(bytes) else {
        return -1;
    };
    with_stats(path, f).unwrap_or(-1)
}

pub fn test_frames(path: &str) -> usize {
    with_stats(path, |s| s.frames).unwrap_or(0)
}

pub fn test_last_id(path: &str) -> u32 {
    with_stats(path, |s| s.last_id).unwrap_or(0)
}

pub fn test_last_list(path: &str) -> usize {
    with_stats(path, |s| s.last_list).unwrap_or(0)
}

pub fn test_last_packet(path: &str) -> usize {
    with_stats(path, |s| s.last_packet).unwrap_or(0)
}
