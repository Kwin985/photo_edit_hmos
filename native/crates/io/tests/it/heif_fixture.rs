//! HEIC opens without the real-file corpus: one small upstream fixture
//! (heic-rs `tests/fixtures/rgb-strips-96.heic`, MIT OR Apache-2.0) checked into
//! `tests/fixtures/`. Feature `heif` only; the corpus suite stays separate.

#![cfg(feature = "heif")]

use photocraft_codecs::{Format, detect};
use photocraft_doc::{ColorMode, SampleType};
use photocraft_io::import;

const HEIC: &[u8] = include_bytes!("../fixtures/rgb-strips-96.heic");

#[test]
fn heic_fixture_opens_as_96x32_rgb_document() {
    assert_eq!(detect(HEIC), Some(Format::Heif));
    let r = import("IMG_0001.HEIC", HEIC).unwrap();
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height), (96, 32));
    assert_eq!((d.mode, d.depth), (ColorMode::Rgb, SampleType::U8));
    assert_eq!(d.layers.len(), 1);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn truncated_heic_fixture_is_an_error_not_a_crash() {
    for cut in [0, 12, 40, HEIC.len() / 2, HEIC.len() - 1] {
        assert!(import("x.heic", &HEIC[..cut]).is_err(), "cut at {cut}");
    }
}