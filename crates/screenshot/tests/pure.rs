//! The pure parts: rectangles and overlay math, pixels (crop, compose, rotation, thumbnails, HDR → SDR), the bytes (PNG, DIB,
//! CF_HDROP), file names and the gallery index format.

use std::path::{Path, PathBuf};

use bu_screenshot::encode::{self, PngLevel};
use bu_screenshot::fake::{mon, pattern};
use bu_screenshot::gallery::{self, Shot};
use bu_screenshot::geom::{self, Rect, Rotation};
use bu_screenshot::image::{self, half_to_f32, HdrToSdr, Image};
use bu_screenshot::naming::{self, LocalTime};

fn numbered(mut v: Vec<geom::Monitor>) -> Vec<geom::Monitor> {
    geom::number_monitors(&mut v);
    v
}

fn noise(w: u32, h: u32, seed: u32) -> Image {
    let mut img = Image::black(w, h);
    let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    for y in 0..h {
        for x in 0..w {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            img.set_pixel(x, y, [s as u8, (s >> 8) as u8, (s >> 16) as u8, 255]);
        }
    }
    img
}

// ---------- geometry ----------

#[test]
fn monitors_are_numbered_left_to_right_then_top_to_bottom() {
    let m = numbered(vec![
        mon(1920, 0, 2560, 1440, false, false),
        mon(-1080, -300, 1080, 1920, false, false),
        mon(0, 0, 1920, 1080, true, false),
    ]);
    let order: Vec<(i32, usize)> = m.iter().map(|m| (m.rect.x, m.number)).collect();
    assert_eq!(order, vec![(-1080, 1), (0, 2), (1920, 3)]);
}

#[test]
fn desktop_bounds_cover_every_monitor_including_negative_ones() {
    let m = numbered(vec![mon(0, 0, 1920, 1080, true, false), mon(-1080, -300, 1080, 1920, false, false)]);
    assert_eq!(geom::desktop_bounds(&m), Rect::new(-1080, -300, 3000, 1920));
}

#[test]
fn monitor_under_the_mouse() {
    let m = numbered(vec![mon(0, 0, 1920, 1080, true, false), mon(1920, 0, 2560, 1440, false, false)]);
    assert_eq!(geom::monitor_at(&m, 0, 0).map(|m| m.number), Some(1));
    assert_eq!(geom::monitor_at(&m, 1919, 1079).map(|m| m.number), Some(1));
    assert_eq!(geom::monitor_at(&m, 1920, 0).map(|m| m.number), Some(2));
    assert_eq!(geom::monitor_at(&m, 100, 1200).map(|m| m.number), None, "the gap under the smaller monitor");
    assert_eq!(geom::monitor_at(&m, -1, 5).map(|m| m.number), None);
}

#[test]
fn preset_is_lit_only_when_the_box_equals_it_exactly() {
    let m = numbered(vec![mon(0, 0, 1920, 1080, true, false), mon(1920, 0, 2560, 1440, false, false)]);
    assert_eq!(geom::preset_matching(&m, &Rect::new(0, 0, 1920, 1080)), Some(1));
    assert_eq!(geom::preset_matching(&m, &Rect::new(1920, 0, 2560, 1440)), Some(2));
    assert_eq!(geom::preset_matching(&m, &Rect::new(0, 0, 4480, 1440)), Some(0));
    assert_eq!(geom::preset_matching(&m, &Rect::new(0, 0, 1919, 1080)), None, "one pixel short is not lit");
    let single = numbered(vec![mon(0, 0, 1920, 1080, true, false)]);
    assert_eq!(geom::preset_matching(&single, &Rect::new(0, 0, 1920, 1080)), Some(1), "one monitor: 1, not All");
}

#[test]
fn typed_size_takes_the_first_two_numbers_with_any_separator() {
    assert_eq!(geom::parse_size("1920x1080"), Some((1920, 1080)));
    assert_eq!(geom::parse_size("1920 × 1080"), Some((1920, 1080)));
    assert_eq!(geom::parse_size(" 800,600 and 5"), Some((800, 600)));
    assert_eq!(geom::parse_size("w=640;h=480"), Some((640, 480)));
    assert_eq!(geom::parse_size("1920"), None);
    assert_eq!(geom::parse_size("abc"), None);
    assert_eq!(geom::parse_size("99999999999999999999 5"), Some((u32::MAX, 5)), "huge numbers saturate, later clamped");
}

#[test]
fn typed_size_clamps_keeps_top_left_and_slides_back() {
    let desk = Rect::new(0, 0, 4480, 1440);
    let cur = Rect::new(100, 50, 300, 200);
    assert_eq!(geom::fit_typed_size(&cur, 1920, 1080, &desk), Rect::new(100, 50, 1920, 1080), "top-left stays");
    assert_eq!(geom::fit_typed_size(&cur, 1, 2, &desk), Rect::new(100, 50, 4, 4), "4 px minimum");
    assert_eq!(geom::fit_typed_size(&cur, 99999, 99999, &desk), desk, "at most the whole desktop");
    let near_edge = Rect::new(4000, 1300, 100, 100);
    assert_eq!(geom::fit_typed_size(&near_edge, 1000, 500, &desk), Rect::new(3480, 940, 1000, 500), "slides back");
    let neg = Rect::new(-1080, -300, 3000, 1920);
    assert_eq!(geom::fit_typed_size(&Rect::new(-2000, -900, 10, 10), 50, 50, &neg), Rect::new(-1080, -300, 50, 50));
}

#[test]
fn dragged_box_is_clamped_and_tiny_boxes_revert() {
    let desk = Rect::new(0, 0, 1920, 1080);
    assert_eq!(geom::clamp_box(&Rect::new(-50, -50, 200, 200), &desk), Some(Rect::new(0, 0, 150, 150)));
    assert_eq!(geom::clamp_box(&Rect::new(10, 10, 3, 100), &desk), None);
    assert_eq!(geom::clamp_box(&Rect::new(10, 10, 4, 4), &desk), Some(Rect::new(10, 10, 4, 4)));
    assert_eq!(geom::clamp_box(&Rect::new(5000, 0, 100, 100), &desk), None);
}

#[test]
fn rect_intersect_and_union() {
    let a = Rect::new(0, 0, 10, 10);
    assert_eq!(a.intersect(&Rect::new(5, 5, 10, 10)), Some(Rect::new(5, 5, 5, 5)));
    assert_eq!(a.intersect(&Rect::new(10, 0, 5, 5)), None, "touching edges do not overlap");
    assert_eq!(a.union(&Rect::new(-5, 20, 5, 5)), Rect::new(-5, 0, 15, 25));
    assert_eq!(Rect::default().union(&a), a);
}

// ---------- pixels ----------

#[test]
fn crop_is_exact_and_refuses_out_of_bounds() {
    let img = noise(64, 48, 1);
    let c = img.crop(&Rect::new(10, 7, 20, 13)).unwrap();
    assert_eq!((c.width, c.height), (20, 13));
    for y in 0..13 {
        for x in 0..20 {
            assert_eq!(c.pixel(x, y), img.pixel(x + 10, y + 7));
        }
    }
    assert!(img.crop(&Rect::new(50, 0, 15, 5)).is_none());
    assert!(img.crop(&Rect::new(-1, 0, 5, 5)).is_none());
    assert!(img.crop(&Rect::new(0, 0, 0, 5)).is_none());
    assert_eq!(img.crop(&Rect::new(0, 0, 64, 48)).unwrap(), img);
}

#[test]
fn compose_places_monitors_side_by_side_and_leaves_gaps_black() {
    let m = numbered(vec![mon(0, 0, 40, 20, true, false), mon(40, 0, 30, 30, false, false)]);
    let (a, b) = (pattern(&m[0], 0), pattern(&m[1], 0));
    let all = image::compose(&Rect::new(0, 0, 70, 30), &[(m[0].rect, &a), (m[1].rect, &b)]);
    assert_eq!((all.width, all.height), (70, 30));
    assert_eq!(all.pixel(0, 0), a.pixel(0, 0));
    assert_eq!(all.pixel(39, 19), a.pixel(39, 19));
    assert_eq!(all.pixel(40, 0), b.pixel(0, 0));
    assert_eq!(all.pixel(69, 29), b.pixel(29, 29));
    assert_eq!(all.pixel(10, 25), [0, 0, 0, 255], "below the shorter monitor = black");
    // A region crossing the seam.
    let r = image::compose(&Rect::new(35, 5, 10, 10), &[(m[0].rect, &a), (m[1].rect, &b)]);
    assert_eq!(r.pixel(4, 0), a.pixel(39, 5));
    assert_eq!(r.pixel(5, 0), b.pixel(0, 5));
}

#[test]
fn rotation_turns_the_picture_the_right_way() {
    // 3×2 picture with distinct pixels.
    let mut img = Image::black(3, 2);
    for y in 0..2 {
        for x in 0..3 {
            img.set_pixel(x, y, [(y * 3 + x) as u8, 0, 0, 255]);
        }
    }
    let id = |i: &Image, x, y| i.pixel(x, y)[0];
    // 90° clockwise: old top-left goes to the top-right; new size 2×3.
    let r90 = img.rotated(Rotation::Cw90);
    assert_eq!((r90.width, r90.height), (2, 3));
    assert_eq!(id(&r90, 1, 0), 0);
    assert_eq!(id(&r90, 0, 0), 3);
    assert_eq!(id(&r90, 1, 2), 2);
    let r180 = img.rotated(Rotation::Cw180);
    assert_eq!(id(&r180, 0, 0), 5);
    assert_eq!(id(&r180, 2, 1), 0);
    let r270 = img.rotated(Rotation::Cw270);
    assert_eq!((r270.width, r270.height), (2, 3));
    assert_eq!(id(&r270, 0, 2), 0, "270°: old top-left goes to the bottom-left");
    // Four quarter turns = the original; 90 then 270 = the original.
    let big = noise(17, 9, 3);
    assert_eq!(big.rotated(Rotation::Cw90).rotated(Rotation::Cw270), big);
    assert_eq!(big.rotated(Rotation::Cw180).rotated(Rotation::Cw180), big);
    assert_eq!(big.rotated(Rotation::None), big);
}

#[test]
fn thumbnail_fits_contain_style_and_averages() {
    let img = noise(1920, 1080, 4);
    let t = img.thumbnail(384, 216);
    assert_eq!((t.width, t.height), (384, 216));
    let tall = noise(1080, 1920, 5).thumbnail(384, 216);
    assert_eq!((tall.width, tall.height), (122, 216), "portrait: height-limited");
    let wide = noise(5120, 1440, 6).thumbnail(384, 216);
    assert_eq!((wide.width, wide.height), (384, 108), "ultrawide pair: width-limited");
    let small = noise(100, 50, 7).thumbnail(384, 216);
    assert_eq!(small, noise(100, 50, 7), "never enlarged");
    // A 2×2 block of known values averages exactly.
    let mut sq = Image::black(2, 2);
    sq.set_pixel(0, 0, [0, 0, 0, 255]);
    sq.set_pixel(1, 0, [100, 10, 1, 255]);
    sq.set_pixel(0, 1, [200, 20, 2, 255]);
    sq.set_pixel(1, 1, [100, 10, 1, 255]);
    assert_eq!(sq.thumbnail(1, 1).pixel(0, 0), [100, 10, 1, 255]);
}

#[test]
fn bgra_rows_with_padding_and_alpha_forced() {
    // 2×2 with a 12-byte pitch (4 bytes padding) and alpha 0 in the source.
    let src = [1, 2, 3, 0, 4, 5, 6, 0, 9, 9, 9, 9, 7, 8, 9, 0, 10, 11, 12, 0, 9, 9, 9, 9];
    let img = image::from_bgra_rows(2, 2, &src, 12);
    assert_eq!(img.bgra, vec![1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]);
}

#[test]
fn half_floats_decode() {
    assert_eq!(half_to_f32(0x0000), 0.0);
    assert_eq!(half_to_f32(0x3c00), 1.0);
    assert_eq!(half_to_f32(0x4000), 2.0);
    assert_eq!(half_to_f32(0x3800), 0.5);
    assert_eq!(half_to_f32(0xbc00), -1.0);
    assert_eq!(half_to_f32(0x7bff), 65504.0);
    assert_eq!(half_to_f32(0x0001), 2f32.powi(-24), "smallest subnormal");
    assert!(half_to_f32(0x7c00).is_infinite());
    assert!(half_to_f32(0x7e00).is_nan());
}

#[test]
fn hdr_to_sdr_divides_by_sdr_white_clips_and_applies_srgb() {
    let at80 = HdrToSdr::new(80.0); // SDR white = scRGB 1.0
    assert_eq!(at80.channel(0x0000), 0);
    assert_eq!(at80.channel(0x3c00), 255, "1.0 = white");
    assert_eq!(at80.channel(0x3800), 188, "linear 0.5 -> sRGB 0.7354 -> 188");
    assert_eq!(at80.channel(0x4000), 255, "HDR highlight above SDR white clips");
    assert_eq!(at80.channel(0xbc00), 0, "negative (out of sRGB gamut) clips to 0");
    assert_eq!(at80.channel(0x7e00), 0, "NaN -> 0");
    let at240 = HdrToSdr::new(240.0); // SDR white = scRGB 3.0 (Windows "SDR content brightness" raised)
    assert_eq!(at240.channel(0x4200), 255, "3.0 = white at 240 nits");
    assert_eq!(at240.channel(0x3e00), 188, "1.5 = half of white");
    assert_eq!(at240.channel(0x3c00), 156, "1.0 = a third of white -> sRGB 0.6125 -> 156");
}

#[test]
fn hdr_rows_convert_with_pitch_and_channel_order() {
    let conv = HdrToSdr::new(80.0);
    // One pixel R=1.0, G=0.5, B=0, A=1, then 8 bytes padding.
    let mut src = Vec::new();
    for h in [0x3c00u16, 0x3800, 0x0000, 0x3c00] {
        src.extend_from_slice(&h.to_le_bytes());
    }
    src.extend_from_slice(&[0xAA; 8]);
    for h in [0u16, 0, 0x3c00, 0x3c00] {
        src.extend_from_slice(&h.to_le_bytes());
    }
    let img = conv.convert_rows(1, 2, &src, 16);
    assert_eq!(img.pixel(0, 0), [0, 188, 255, 255], "BGRA order");
    assert_eq!(img.pixel(0, 1), [255, 0, 0, 255]);
}

#[test]
fn non_black_count() {
    let mut img = Image::black(4, 4);
    assert_eq!(img.non_black_pixels(), 0);
    img.set_pixel(1, 1, [0, 0, 1, 255]);
    img.set_pixel(2, 3, [9, 0, 0, 255]);
    assert_eq!(img.non_black_pixels(), 2);
}

// ---------- bytes ----------

#[test]
fn png_is_lossless_rgb_at_every_level() {
    let img = noise(97, 61, 8);
    for level in [PngLevel::Fast, PngLevel::Balanced, PngLevel::High] {
        let png = encode::png_bytes(&img, level).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(png[25], 2, "IHDR colour type 2 = RGB (no alpha)");
        assert_eq!(png[24], 8, "8 bits per channel");
        assert_eq!(encode::decode_png(&png).unwrap(), img, "{level:?}");
    }
}

#[test]
fn png_decode_refuses_garbage() {
    assert!(encode::decode_png(b"not a png").is_err());
}

#[test]
fn dib_header_and_bottom_up_rows() {
    let mut img = Image::black(2, 2);
    img.set_pixel(0, 0, [1, 2, 3, 255]);
    img.set_pixel(1, 1, [7, 8, 9, 255]);
    let d = encode::dib_bytes(&img);
    assert_eq!(d.len(), 40 + 16);
    let u32_at = |i: usize| u32::from_le_bytes(d[i..i + 4].try_into().unwrap());
    assert_eq!(u32_at(0), 40, "biSize");
    assert_eq!(u32_at(4), 2, "biWidth");
    assert_eq!(u32_at(8) as i32, 2, "biHeight positive = bottom-up");
    assert_eq!(u16::from_le_bytes([d[12], d[13]]), 1, "planes");
    assert_eq!(u16::from_le_bytes([d[14], d[15]]), 32, "bits");
    assert_eq!(u32_at(16), 0, "BI_RGB");
    assert_eq!(u32_at(20), 16, "image size");
    // First stored row = the picture's bottom row.
    assert_eq!(&d[40..48], &[0, 0, 0, 255, 7, 8, 9, 255]);
    assert_eq!(&d[48..56], &[1, 2, 3, 255, 0, 0, 0, 255]);
}

#[test]
fn dropfiles_layout() {
    let d = encode::dropfiles_bytes(&[PathBuf::from(r"C:\a b\x.png"), PathBuf::from(r"D:\y.png")]);
    let u32_at = |i: usize| u32::from_le_bytes(d[i..i + 4].try_into().unwrap());
    assert_eq!(u32_at(0), 20, "pFiles");
    assert_eq!(u32_at(16), 1, "fWide");
    let w: Vec<u16> = d[20..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let text = String::from_utf16(&w).unwrap();
    assert_eq!(text, "C:\\a b\\x.png\0D:\\y.png\0\0");
}

// ---------- names ----------

#[test]
fn file_name_follows_design() {
    let t = LocalTime { year: 2026, month: 3, day: 7, hour: 9, minute: 5, second: 59 };
    assert_eq!(naming::base_name(&t), "Screenshot 2026-03-07 09-05");
}

#[test]
fn same_minute_gets_a_number() {
    let t = LocalTime { year: 2026, month: 10, day: 8, hour: 1, minute: 36, second: 0 };
    let dir = Path::new(r"C:\shots");
    let taken: Vec<PathBuf> =
        vec![dir.join("Screenshot 2026-10-08 01-36.png"), dir.join("Screenshot 2026-10-08 01-36 (2).png")];
    assert_eq!(naming::free_path(dir, &t, |_| false), dir.join("Screenshot 2026-10-08 01-36.png"));
    assert_eq!(naming::free_path(dir, &t, |p| taken[..1].iter().any(|x| x == p)), dir.join("Screenshot 2026-10-08 01-36 (2).png"));
    assert_eq!(naming::free_path(dir, &t, |p| taken.iter().any(|x| x == p)), dir.join("Screenshot 2026-10-08 01-36 (3).png"));
}

// ---------- gallery index ----------

#[test]
fn index_round_trip_newest_first() {
    let shots = vec![
        Shot { id: 10, path: PathBuf::from(r"C:\a\Screenshot 1.png"), width: 1920, height: 1080 },
        Shot { id: 30, path: PathBuf::from(r"D:\other place\Screenshot 2 (2).png"), width: 4480, height: 1440 },
        Shot { id: 20, path: PathBuf::from(r"\\server\share\s.png"), width: 4, height: 4 },
    ];
    let text = gallery::render(&shots);
    assert!(text.starts_with("bu-screenshot index 1\n"));
    let back = gallery::parse(&text).unwrap();
    assert_eq!(back.iter().map(|s| s.id).collect::<Vec<_>>(), vec![30, 20, 10]);
    assert_eq!(back[0].path, PathBuf::from(r"D:\other place\Screenshot 2 (2).png"));
    assert_eq!((back[0].width, back[0].height), (4480, 1440));
    assert_eq!(gallery::parse("").unwrap(), vec![]);
}

#[test]
fn index_refuses_damaged_files() {
    assert!(gallery::parse("something else\n").is_err());
    assert!(gallery::parse("bu-screenshot index 1\n12\t5\n").is_err());
    assert!(gallery::parse("bu-screenshot index 1\nx\t5\t5\tC:\\a.png\n").is_err());
}

#[test]
fn ids_are_unique_even_in_the_same_millisecond() {
    let shots = vec![Shot { id: 500, path: PathBuf::new(), width: 1, height: 1 }];
    assert_eq!(gallery::new_id(&shots, 900), 900);
    assert_eq!(gallery::new_id(&shots, 500), 501);
    assert_eq!(gallery::new_id(&shots, 100), 501, "clock went back: still after the newest");
    assert_eq!(gallery::new_id(&[], 7), 7);
}
