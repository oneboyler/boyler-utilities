//! The finished picture (coFinish / mkShot): the box's pixels out of the frozen picture, exactly (no resampling), with the
//! marks drawn on top and clipped to the box.

use bu_screenshot::{Frozen, Image, Rect};
use skia_safe as sk;

use super::ink;
use super::model::Ann;

/// A bu-screenshot picture as a Skia image (BGRA, opaque).
pub fn to_sk(img: &Image) -> Option<sk::Image> {
    let info = sk::ImageInfo::new((img.width as i32, img.height as i32), sk::ColorType::BGRA8888, sk::AlphaType::Opaque, None);
    sk::images::raster_from_data(&info, sk::Data::new_copy(&img.bgra), img.width as usize * 4)
}

/// Skia pixels back into a bu-screenshot picture.
fn from_surface(s: &mut sk::Surface, w: u32, h: u32) -> Option<Image> {
    let info = sk::ImageInfo::new((w as i32, h as i32), sk::ColorType::BGRA8888, sk::AlphaType::Premul, None);
    let mut bgra = vec![0u8; w as usize * h as usize * 4];
    if !s.read_pixels(&info, &mut bgra, w as usize * 4, (0, 0)) {
        return None;
    }
    for px in bgra.as_chunks_mut::<4>().0 {
        px[3] = 255;
    }
    Some(Image { width: w, height: h, bgra })
}

/// The box `r` (desktop px) of the frozen picture with every mark. Without marks the pixels are the frozen ones, untouched.
pub fn compose(frozen: &Frozen, r: &Rect, anns: &[Ann]) -> bu_screenshot::Result<Image> {
    let base = frozen.crop(r)?;
    if anns.is_empty() {
        return Ok(base);
    }
    let fail = || bu_screenshot::Error::BadData("drawing the marks failed".into());
    let src = to_sk(&base).ok_or_else(fail)?;
    let info = sk::ImageInfo::new((r.w as i32, r.h as i32), sk::ColorType::BGRA8888, sk::AlphaType::Premul, None);
    let mut s = sk::surfaces::raster(&info, None, None).ok_or_else(fail)?;
    let cv = s.canvas();
    cv.draw_image(&src, (0, 0), None);
    cv.translate((-(r.x as f32), -(r.y as f32)));
    ink::draw_all(cv, anns, sk::Rect::from_xywh(r.x as f32, r.y as f32, r.w as f32, r.h as f32), None);
    from_surface(&mut s, r.w, r.h).ok_or_else(fail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::screenshots::overlay::model::Ann;
    use bu_screenshot::fake::FakeOs;
    use bu_screenshot::Screenshots;

    fn frozen() -> Frozen {
        let dir = std::env::temp_dir().join("bu-019q-never-written");
        Screenshots::new(FakeOs::two_monitors(), dir).capture_all().unwrap()
    }

    #[test]
    fn no_marks_is_the_frozen_pixels_exactly() {
        let f = frozen();
        let r = Rect::new(1900, 10, 60, 40);
        assert_eq!(compose(&f, &r, &[]).unwrap(), f.crop(&r).unwrap());
    }

    #[test]
    fn marks_are_drawn_and_clipped_to_the_box() {
        let f = frozen();
        let r = Rect::new(100, 100, 200, 100);
        // a red box crossing the right edge: inside it changes pixels; nothing is drawn outside (the picture is just the box)
        let a = Ann::Box { c: 0xff453a, a: (150.0, 120.0), b: (400.0, 180.0), s: 1.0 };
        let img = compose(&f, &r, &[a]).unwrap();
        let base = f.crop(&r).unwrap();
        assert_eq!((img.width, img.height), (200, 100));
        let p = img.pixel(50, 20);
        assert!(p[2] > 200 && p[1] < 120, "the stroke is red: {p:?}");
        assert_eq!(img.pixel(10, 90), base.pixel(10, 90), "away from the mark nothing changes");
        assert!(img.bgra.chunks(4).all(|c| c[3] == 255));
    }

    #[test]
    fn a_box_off_the_picture_is_refused() {
        let f = frozen();
        assert!(compose(&f, &Rect::new(4470, 0, 20, 20), &[]).is_err());
    }
}
