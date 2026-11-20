//! Pictures for import and export: JPEG/WebP/GIF avatars become PNG (the card formats can only
//! carry a PNG), and every avatar gets a small WebP thumbnail for grids and lists.
//!
//! `image` (pure Rust) instead of libwebp: lossless WebP is a little larger than lossy, but
//! there is no C library to build for Windows, Android and iOS.

use std::io::Cursor;
use std::sync::{Condvar, Mutex, MutexGuard};

use image::{DynamicImage, ImageFormat, ImageReader, Limits};

/// Width of a thumbnail in pixels (two columns of a phone grid at 2x, a desktop card at 1x).
pub const THUMB_WIDTH: u32 = 384;

/// Largest picture decoded at all (side length and memory), so a tiny file that claims to be
/// huge cannot take the RAM. 192 MiB at 4 bytes per pixel: a 48-megapixel phone photo (backdrop).
const MAX_SIDE: u32 = 12_000;
pub const MAX_ALLOC: u64 = 192 << 20;
/// Bytes of decoded pictures in memory at the same time in the whole program, whatever asks for
/// it (imports of many files, links, backdrops): a picture takes 8 bytes per pixel (decoded RGBA
/// plus a converted copy). Card portraits take a few MB each and run side by side; the largest
/// pictures wait for each other.
pub const DECODE_BUDGET: u64 = 512 << 20;

static IN_USE: Mutex<u64> = Mutex::new(0);
static FREED: Condvar = Condvar::new();

fn in_use() -> MutexGuard<'static, u64> {
    IN_USE.lock().unwrap_or_else(|p| p.into_inner())
}

/// Room in [`DECODE_BUDGET`], held while a decoded picture is in memory.
struct Slot(u64);

impl Slot {
    fn take(bytes: u64) -> Slot {
        let bytes = bytes.min(DECODE_BUDGET);
        let mut used = in_use();
        while *used + bytes > DECODE_BUDGET {
            used = FREED.wait(used).unwrap_or_else(|p| p.into_inner());
        }
        *used += bytes;
        #[cfg(test)]
        tests::PEAK.fetch_max(*used, std::sync::atomic::Ordering::SeqCst);
        Slot(bytes)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        *in_use() -= self.0;
        FREED.notify_all();
    }
}

/// The picture, decoded once the budget has room for it (the room is held as long as the slot).
/// None: unreadable, or over [`MAX_SIDE`] / [`MAX_ALLOC`] (known from the header, so such a
/// picture is never decoded).
fn decode(bytes: &[u8]) -> Option<(Slot, DynamicImage)> {
    let reader = || -> Option<ImageReader<Cursor<&[u8]>>> {
        let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_SIDE);
        limits.max_image_height = Some(MAX_SIDE);
        limits.max_alloc = Some(MAX_ALLOC);
        reader.limits(limits);
        Some(reader)
    };
    let (w, h) = reader()?.into_dimensions().ok()?;
    if w > MAX_SIDE || h > MAX_SIDE || u64::from(w) * u64::from(h) * 4 > MAX_ALLOC {
        return None;
    }
    let slot = Slot::take(u64::from(w) * u64::from(h) * 8);
    let img = reader()?.decode().ok()?;
    Some((slot, img))
}

fn encode(img: &DynamicImage, format: ImageFormat) -> Option<Vec<u8>> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, format).ok()?;
    Some(out.into_inner())
}

/// The picture as PNG: a PNG stays byte for byte as it is, JPEG/WebP/GIF are converted (a GIF
/// keeps its first frame). None: not a picture that can be read.
pub fn to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(bytes.to_vec());
    }
    let (_slot, img) = decode(bytes)?;
    // PNG has no CMYK or 16-bit float; RGBA8 is what every app reads.
    encode(&DynamicImage::ImageRgba8(img.into_rgba8()), ImageFormat::Png)
}

/// Does this decode as a whole picture within the limits of [`decode`]? (A fetched picture is
/// handed on byte for byte only when it does.)
pub fn readable(bytes: &[u8]) -> bool {
    decode(bytes).is_some()
}

/// Lossless WebP, at most [`THUMB_WIDTH`] wide (smaller pictures keep their size).
pub fn thumb_webp(bytes: &[u8]) -> Option<Vec<u8>> {
    let (_slot, img) = decode(bytes)?;
    let img = if img.width() > THUMB_WIDTH {
        let height = (u64::from(img.height()) * u64::from(THUMB_WIDTH) / u64::from(img.width())).max(1) as u32;
        img.resize_exact(THUMB_WIDTH, height, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    // The WebP encoder takes 8-bit RGB(A) only.
    let img = if img.color().has_alpha() {
        DynamicImage::ImageRgba8(img.to_rgba8())
    } else {
        DynamicImage::ImageRgb8(img.to_rgb8())
    };
    encode(&img, ImageFormat::WebP)
}

/// A backdrop: JPEG, its longer side at most `max_side` (smaller pictures keep their size). A GIF
/// keeps its first frame; transparency is dropped. None: not a picture that can be read.
pub fn fit_jpeg(bytes: &[u8], max_side: u32, quality: u8) -> Option<Vec<u8>> {
    let (_slot, img) = decode(bytes)?;
    let img = if img.width().max(img.height()) > max_side { img.resize(max_side, max_side, image::imageops::FilterType::Triangle) } else { img };
    let mut out = Vec::new();
    img.to_rgb8().write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: u32, h: u32, format: ImageFormat) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
        encode(&DynamicImage::ImageRgb8(img), format).unwrap()
    }

    #[test]
    fn jpeg_becomes_png_and_png_stays() {
        let jpeg = picture(40, 30, ImageFormat::Jpeg);
        let png = to_png(&jpeg).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        assert_eq!(image::load_from_memory(&png).unwrap().width(), 40);
        assert_eq!(to_png(&png).unwrap(), png);
        assert_eq!(to_png(b"not a picture"), None);
    }

    #[test]
    fn thumbnail_is_webp_and_at_most_384_wide() {
        let big = picture(1000, 1500, ImageFormat::Png);
        let thumb = thumb_webp(&big).unwrap();
        assert_eq!(&thumb[8..12], b"WEBP");
        let t = image::load_from_memory(&thumb).unwrap();
        assert_eq!((t.width(), t.height()), (384, 576));
        let small = thumb_webp(&picture(100, 80, ImageFormat::Jpeg)).unwrap();
        assert_eq!(image::load_from_memory(&small).unwrap().width(), 100);
    }

    #[test]
    fn backdrop_is_jpeg_and_at_most_the_given_side() {
        let jpeg = fit_jpeg(&picture(3000, 1500, ImageFormat::Png), 1000, 80).unwrap();
        assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]));
        let img = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((img.width(), img.height()), (1000, 500));
        let small = fit_jpeg(&picture(64, 48, ImageFormat::WebP), 1000, 80).unwrap();
        assert_eq!(image::load_from_memory(&small).unwrap().width(), 64);
        assert_eq!(fit_jpeg(b"not a picture", 1000, 80), None);
    }

    #[test]
    fn huge_claimed_size_is_refused() {
        // A PNG header that claims 100 000 x 100 000 pixels.
        let mut png = picture(1, 1, ImageFormat::Png);
        png[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        png[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        assert_eq!(thumb_webp(&png), None);
    }

    /// A real picture of 7 500 x 7 500 pixels (a small file: one colour) would need 225 MB as
    /// RGBA: refused by the memory limit, not decoded. A 4 000 x 4 000 one still works.
    #[test]
    fn huge_real_picture_is_refused() {
        let encode_rgba = |side: u32| {
            let img = image::RgbaImage::from_pixel(side, side, image::Rgba([90, 120, 150, 255]));
            encode(&DynamicImage::ImageRgba8(img), ImageFormat::Png).unwrap()
        };
        let huge = encode_rgba(7_500);
        assert!(huge.len() < 2 << 20, "the file itself is small: {}", huge.len());
        assert_eq!(thumb_webp(&huge), None);
        assert_eq!(fit_jpeg(&huge, 1000, 80), None);
        assert!(thumb_webp(&encode_rgba(4_000)).is_some());
    }

    pub(super) static PEAK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// However many threads ask at once, decoded pictures never hold more than the budget: eight
    /// 4000 x 4000 pictures (128 MB each) go at most four at a time.
    #[test]
    fn decoding_is_limited_program_wide() {
        let pic = picture(4000, 4000, ImageFormat::Png);
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| assert!(thumb_webp(&pic).is_some()));
            }
        });
        let peak = PEAK.load(std::sync::atomic::Ordering::SeqCst);
        assert!((4000 * 4000 * 8..=DECODE_BUDGET).contains(&peak), "peak {peak}");
    }
}
