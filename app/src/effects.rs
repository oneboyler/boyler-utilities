//! Direct2D effect descriptions for Windows.UI.Composition (the glass): a tiny IGraphicsEffect implementation
//! that hands the compositor a D2D effect CLSID, its properties and its sources.

use std::cell::RefCell;
use windows::core::*;
use windows::Foundation::{IPropertyValue, PropertyValue};
use windows::Graphics::Effects::*;
use windows::Win32::System::WinRT::Graphics::Direct2D::*;

pub enum Prop {
    F(f32),
    U(u32),
    B(bool),
    Arr(Vec<f32>),
}

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
pub struct Effect {
    clsid: GUID,
    props: Vec<Prop>,
    sources: Vec<IGraphicsEffectSource>,
    name: RefCell<HSTRING>,
}

impl Effect {
    pub fn make(clsid: GUID, props: Vec<Prop>, sources: Vec<IGraphicsEffectSource>) -> IGraphicsEffect {
        Effect { clsid, props, sources, name: RefCell::new(HSTRING::new()) }.into()
    }
}

impl IGraphicsEffect_Impl for Effect_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(self.name.borrow().clone())
    }
    fn SetName(&self, name: &HSTRING) -> Result<()> {
        *self.name.borrow_mut() = name.clone();
        Ok(())
    }
}

impl IGraphicsEffectSource_Impl for Effect_Impl {}

impl IGraphicsEffectD2D1Interop_Impl for Effect_Impl {
    fn GetEffectId(&self) -> Result<GUID> {
        Ok(self.clsid)
    }
    fn GetNamedPropertyMapping(&self, _name: &PCWSTR, _index: *mut u32, _mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING) -> Result<()> {
        Err(windows::Win32::Foundation::E_INVALIDARG.into())
    }
    fn GetPropertyCount(&self) -> Result<u32> {
        Ok(self.props.len() as u32)
    }
    fn GetProperty(&self, index: u32) -> Result<IPropertyValue> {
        let p = self.props.get(index as usize).ok_or_else(|| Error::from(windows::Win32::Foundation::E_INVALIDARG))?;
        let v: IInspectable = match p {
            Prop::F(v) => PropertyValue::CreateSingle(*v)?,
            Prop::U(v) => PropertyValue::CreateUInt32(*v)?,
            Prop::B(v) => PropertyValue::CreateBoolean(*v)?,
            Prop::Arr(a) => PropertyValue::CreateSingleArray(a)?,
        };
        v.cast()
    }
    fn GetSource(&self, index: u32) -> Result<IGraphicsEffectSource> {
        self.sources.get(index as usize).cloned().ok_or_else(|| windows::Win32::Foundation::E_INVALIDARG.into())
    }
    fn GetSourceCount(&self) -> Result<u32> {
        Ok(self.sources.len() as u32)
    }
}

pub const CLSID_GAUSSIAN_BLUR: GUID = GUID::from_u128(0x1feb6d69_2fe6_4ac9_8c58_1d7f93e7a6a5);
pub const CLSID_COLOR_MATRIX: GUID = GUID::from_u128(0x921f03d6_641c_47df_852d_b4bb6153ae11);
pub const CLSID_BORDER: GUID = GUID::from_u128(0x2a2d49c0_4acf_43c7_8c6a_7c4a27874d27);
pub const CLSID_BLEND: GUID = GUID::from_u128(0x81c5b77b_13f8_4cdd_ad20_c890547ac65d);
pub const CLSID_FLOOD: GUID = GUID::from_u128(0x61c23c20_ae69_4d8e_94cf_50078df638f2);
/// D2D1_BLEND_MODE_DARKEN / _LIGHTEN (2 / 3 - 1 is SCREEN, 4 DISSOLVE; checked by d2dref's test on Direct2D itself)
pub const BLEND_DARKEN: u32 = 2;
pub const BLEND_LIGHTEN: u32 = 3;

/// Order 041 (adaptive glass): every colour channel of `src` kept at most (`BLEND_DARKEN`) or at least (`BLEND_LIGHTEN`)
/// `level` (0..1) - a D2D Blend of `src` with a flat grey (D2D Flood). Opaque inputs: darken = min, lighten = max.
pub fn clamp_level(mode: u32, level: f32, src: IGraphicsEffectSource) -> Result<IGraphicsEffect> {
    let flood = Effect::make(CLSID_FLOOD, vec![Prop::Arr(vec![level, level, level, 1.0])], vec![]);
    Ok(Effect::make(CLSID_BLEND, vec![Prop::U(mode)], vec![src, flood.cast()?]))
}
/// Gaussian blur at full quality (no downscaling), hard border - option 3 of Q_013_01.
pub fn gaussian_blur_quality(sigma: f32, src: IGraphicsEffectSource) -> IGraphicsEffect {
    // props: StandardDeviation, Optimization (quality = 2), BorderMode (hard = 1)
    Effect::make(CLSID_GAUSSIAN_BLUR, vec![Prop::F(sigma), Prop::U(2), Prop::U(1)], vec![src])
}

/// Extend the image's edges by mirroring (Chromium's backdrop-filter edge mode), so the blur has no dark rim.
pub fn mirror_edges(src: IGraphicsEffectSource) -> IGraphicsEffect {
    // props: EdgeModeX, EdgeModeY (mirror = 2)
    Effect::make(CLSID_BORDER, vec![Prop::U(2), Prop::U(2)], vec![src])
}

/// Gaussian blur, `sigma` = standard deviation in px (CSS `blur(r)` = sigma r). Hard border = edges extend, no dark fringe.
pub fn gaussian_blur(sigma: f32, src: IGraphicsEffectSource) -> IGraphicsEffect {
    // props: StandardDeviation, Optimization (balanced = 1), BorderMode (hard = 1)
    Effect::make(CLSID_GAUSSIAN_BLUR, vec![Prop::F(sigma), Prop::U(1), Prop::U(1)], vec![src])
}

/// CSS `saturate(s)` as a D2D colour matrix (rows = inputs R G B A 1, columns = outputs R G B A).
pub fn saturate(s: f32, src: IGraphicsEffectSource) -> IGraphicsEffect {
    let m = saturate_matrix(s);
    // props: Matrix, AlphaMode (premultiplied = 1), ClampOutput
    Effect::make(CLSID_COLOR_MATRIX, vec![Prop::Arr(m.to_vec()), Prop::U(1), Prop::B(true)], vec![src])
}

/// CSS `saturate(s) brightness(b)` folded into one colour matrix (brightness scales the saturated RGB).
pub fn saturate_brightness(s: f32, b: f32, src: IGraphicsEffectSource) -> IGraphicsEffect {
    let mut m = saturate_matrix(s);
    for row in 0..3 {
        for col in 0..3 {
            m[row * 4 + col] *= b;
        }
    }
    Effect::make(CLSID_COLOR_MATRIX, vec![Prop::Arr(m.to_vec()), Prop::U(1), Prop::B(true)], vec![src])
}

pub fn saturate_matrix(s: f32) -> [f32; 20] {
    let rr = 0.213 + 0.787 * s;
    let rg = 0.715 - 0.715 * s;
    let rb = 0.072 - 0.072 * s;
    let gr = 0.213 - 0.213 * s;
    let gg = 0.715 + 0.285 * s;
    let gb = 0.072 - 0.072 * s;
    let br = 0.213 - 0.213 * s;
    let bg = 0.715 - 0.715 * s;
    let bb = 0.072 + 0.928 * s;
    // D2D_MATRIX_5X4_F: row i = contribution of input channel i to (R,G,B,A)
    [
        rr, gr, br, 0.0, // input R
        rg, gg, bg, 0.0, // input G
        rb, gb, bb, 0.0, // input B
        0.0, 0.0, 0.0, 1.0, // input A
        0.0, 0.0, 0.0, 0.0, // offset
    ]
}
