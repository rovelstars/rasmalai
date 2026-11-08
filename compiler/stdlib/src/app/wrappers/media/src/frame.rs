//! Raw RGBA frames for the media ABI.
//!
//! A frame is a 24-byte header followed by tightly packed RGBA8 pixels:
//!
//! ```text
//! u32 magic 0x524E5852 ("RNXR")
//! u32 width
//! u32 height
//! u32 stride (width * 4)
//! u32 format (0 = RGBA8, straight alpha)
//! u32 length (pixel bytes)
//! ```
//!
//! The display-list image path recognizes the same header, so a decoded
//! video or camera frame can be passed straight to `Image`.

pub const MAGIC: u32 = 0x5258_4E52;
pub const HEADER: usize = 24;

pub fn pack(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let stride = width as usize * 4;
    let mut out = Vec::with_capacity(HEADER + pixels.len());
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&(stride as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
    out.extend_from_slice(pixels);
    out
}

pub fn is_frame(bytes: &[u8]) -> bool {
    bytes.len() >= HEADER && u32::from_le_bytes(bytes[0..4].try_into().unwrap()) == MAGIC
}

pub fn parse_header(bytes: &[u8]) -> Option<(u32, u32, usize)> {
    if !is_frame(bytes) {
        return None;
    }
    let width = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
    let height = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
    let stride = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
    let format = u32::from_le_bytes(bytes[16..20].try_into().ok()?);
    let len = u32::from_le_bytes(bytes[20..24].try_into().ok()?) as usize;
    if width == 0 || height == 0 || format != 0 || stride < width as usize * 4 {
        return None;
    }
    if bytes.len() < HEADER + len {
        return None;
    }
    Some((width, height, len))
}
