use crate::acp;
use base64::{Engine, engine::general_purpose::STANDARD};
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_IMAGES: usize = 8;
pub const MAX_TOTAL_IMAGE_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_IMAGE_PIXELS: u64 = 16_777_216;
#[derive(Clone, Debug)]
pub struct PromptImage {
    pub mime_type: String,
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}
impl PromptImage {
    pub fn validate(data: Vec<u8>) -> Result<Self, String> {
        if data.is_empty() || data.len() > MAX_IMAGE_BYTES {
            return Err("Images must be at most 5 MiB".into());
        }
        let (mime, w, h) = dimensions(&data).ok_or("Invalid or unsupported image")?;
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
            return Err("Image dimensions exceed 16 million pixels".into());
        }
        let mut reader = image::ImageReader::new(std::io::Cursor::new(&data))
            .with_guessed_format()
            .map_err(|_| "Invalid image header")?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        reader
            .decode()
            .map_err(|_| "Invalid image data or decoding limit exceeded")?;
        Ok(Self {
            mime_type: mime.into(),
            data,
            width: w,
            height: h,
        })
    }
    pub fn content(&self) -> acp::ContentBlock {
        acp::ContentBlock::Image(acp::ImageContent::new(
            STANDARD.encode(&self.data),
            self.mime_type.clone(),
        ))
    }
}
pub fn validate_collection(images: &[PromptImage]) -> Result<(), String> {
    if images.len() > MAX_IMAGES
        || images.iter().map(|v| v.data.len()).sum::<usize>() > MAX_TOTAL_IMAGE_BYTES
    {
        Err("At most 8 images and 20 MiB total".into())
    } else {
        Ok(())
    }
}
fn dimensions(b: &[u8]) -> Option<(&'static str, u32, u32)> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") && b.len() >= 33 && &b[12..16] == b"IHDR" {
        return Some((
            "image/png",
            u32::from_be_bytes(b[16..20].try_into().ok()?),
            u32::from_be_bytes(b[20..24].try_into().ok()?),
        ));
    }
    if (b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")) && b.len() >= 13 {
        return Some((
            "image/gif",
            u16::from_le_bytes(b[6..8].try_into().ok()?).into(),
            u16::from_le_bytes(b[8..10].try_into().ok()?).into(),
        ));
    }
    if b.starts_with(b"RIFF") && b.len() >= 30 && &b[8..12] == b"WEBP" {
        if &b[12..16] == b"VP8X" {
            let n = |i: usize| {
                1 + u32::from(b[i]) + (u32::from(b[i + 1]) << 8) + (u32::from(b[i + 2]) << 16)
            };
            return Some(("image/webp", n(24), n(27)));
        }
        if &b[12..16] == b"VP8 " && b.len() >= 30 && &b[23..26] == b"\x9d\x01\x2a" {
            return Some((
                "image/webp",
                u32::from(u16::from_le_bytes(b[26..28].try_into().ok()?) & 0x3fff),
                u32::from(u16::from_le_bytes(b[28..30].try_into().ok()?) & 0x3fff),
            ));
        }
        if &b[12..16] == b"VP8L" && b.len() >= 25 && b[20] == 0x2f {
            let bits = u32::from_le_bytes(b[21..25].try_into().ok()?);
            return Some((
                "image/webp",
                (bits & 0x3fff) + 1,
                ((bits >> 14) & 0x3fff) + 1,
            ));
        }
    }
    if b.starts_with(b"\xff\xd8") {
        let mut i = 2;
        while i + 4 <= b.len() {
            if b[i] != 0xff {
                return None;
            }
            let marker = b[i + 1];
            i += 2;
            if marker == 0xff {
                i -= 1;
                continue;
            }
            if marker == 0xd9 || marker == 0xda {
                return None;
            }
            if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
                continue;
            }
            let len = usize::from(u16::from_be_bytes(b[i..i + 2].try_into().ok()?));
            if len < 2 || i + len > b.len() {
                return None;
            }
            if [
                0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf,
            ]
            .contains(&marker)
                && len >= 8
            {
                return Some((
                    "image/jpeg",
                    u16::from_be_bytes(b[i + 5..i + 7].try_into().ok()?).into(),
                    u16::from_be_bytes(b[i + 3..i + 5].try_into().ok()?).into(),
                ));
            }
            i += len;
        }
    }
    None
}
