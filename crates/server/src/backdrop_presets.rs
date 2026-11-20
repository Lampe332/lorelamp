//! Ready-made backdrop pictures that ship inside the program (`crates/server/assets/backdrops/`):
//! the full picture (WebP, 1672×941) and a small thumbnail (WebP, 400×225) for the gallery. Picking
//! one stores a JPEG copy as the chat's or character's backdrop, exactly like an uploaded picture,
//! and remembers which preset it was (see `store::looks`).

macro_rules! presets {
    ($($id:literal),* $(,)?) => {
        /// `(id, picture, thumbnail)` in gallery order.
        pub const PRESETS: &[(&str, &[u8], &[u8])] = &[$((
            $id,
            include_bytes!(concat!("../assets/backdrops/", $id, ".webp")),
            include_bytes!(concat!("../assets/backdrops/thumbs/", $id, ".webp")),
        )),*];
    };
}

presets!(
    "taverne",
    "bibliothek",
    "wald",
    "burg",
    "strand",
    "stadt-nacht",
    "raumstation",
    "cafe",
    "kerker",
    "bergsee",
    "japanische-gasse",
    "zimmer",
    "wueste",
    "hafen",
    "schneedorf",
    "magierturm",
);

fn find(id: &str) -> Option<&'static (&'static str, &'static [u8], &'static [u8])> {
    PRESETS.iter().find(|p| p.0 == id)
}

/// The full picture of a preset.
pub fn picture(id: &str) -> Option<&'static [u8]> {
    find(id).map(|p| p.1)
}

/// The gallery thumbnail of a preset.
pub fn thumb(id: &str) -> Option<&'static [u8]> {
    find(id).map(|p| p.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_a_readable_picture_with_a_small_thumbnail() {
        assert_eq!(PRESETS.len(), 16);
        for (id, full, small) in PRESETS {
            assert!(crate::store::image_mime(full) == Some("image/webp"), "{id}");
            assert!(full.len() <= 256 * 1024, "{id} is {} bytes", full.len());
            assert!(small.len() <= 24 * 1024, "{id} thumb is {} bytes", small.len());
            assert!(crate::imaging::fit_jpeg(small, 2560, 86).is_some(), "{id} thumb decodes");
        }
        assert!(picture("taverne").is_some() && thumb("magierturm").is_some());
        assert!(picture("../rp-app.db").is_none() && thumb("").is_none());
    }
}
