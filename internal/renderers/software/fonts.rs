// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::{Fixed, PhysicalLength, PhysicalSize};
use crate::{GlyphIdStrategy, GlyphProvider};
use i_slint_core::Coord;
use i_slint_core::graphics::{BitmapFont, FontRequest};
use i_slint_core::lengths::{LogicalLength, ScaleFactor};
use i_slint_core::textlayout::TextLayout;

i_slint_core::thread_local! {
    static BITMAP_FONTS: RefCell<Vec<&'static BitmapFont>> = RefCell::default()
}

/// A glyph alpha map provided to the software renderer.
#[derive(derive_more::From, Clone, Debug)]
pub enum GlyphAlphaMap {
    /// Borrowed alpha map data.
    Static(&'static [u8]),
    /// Shared alpha map data owned by the provider.
    Shared(Rc<[u8]>),
}

impl GlyphAlphaMap {
    /// Returns the number of bytes in the alpha map.
    pub fn len(&self) -> usize {
        match self {
            Self::Static(data) => data.len(),
            Self::Shared(data) => data.len(),
        }
    }

    /// Returns true if the alpha map has no data.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone)]
pub struct RenderableGlyph {
    pub x: Fixed<i32, 8>,
    pub y: Fixed<i32, 8>,
    pub width: PhysicalLength,
    pub height: PhysicalLength,
    pub alpha_map: GlyphAlphaMap,
    pub pixel_stride: u16,
    pub sdf: bool,
}

impl RenderableGlyph {
    pub fn size(&self) -> PhysicalSize {
        PhysicalSize::from_lengths(self.width, self.height)
    }
}

// Subset of `RenderableGlyph`, specfically for VectorFonts.
#[cfg(feature = "systemfonts")]
#[derive(Clone)]
pub struct RenderableVectorGlyph {
    pub x: Fixed<i32, 8>,
    pub y: Fixed<i32, 8>,
    pub width: PhysicalLength,
    pub height: PhysicalLength,
    pub alpha_map: Rc<[u8]>,
    pub pixel_stride: u16,
    pub bounds: fontdue::OutlineBounds,
}

#[cfg(feature = "systemfonts")]
impl RenderableVectorGlyph {
    pub fn size(&self) -> PhysicalSize {
        PhysicalSize::from_lengths(self.width, self.height)
    }
}

pub trait GlyphRenderer {
    fn render_glyph(&self, glyph_id: core::num::NonZeroU16) -> Option<RenderableGlyph>;
    /// The amount of pixel in the original image that correspond to one pixel in the rendered image
    fn scale_delta(&self) -> Fixed<u16, 8>;
}

pub(super) const DEFAULT_FONT_SIZE: LogicalLength = LogicalLength::new(12 as Coord);

mod pixelfont;
mod font_service;
mod dynamicfont;
#[cfg(feature = "systemfonts")]
pub mod vectorfont;

#[cfg(feature = "systemfonts")]
pub mod systemfonts;

pub(crate) use dynamicfont::DynamicFontCache;
pub(crate) use font_service::FontService;

#[derive(derive_more::From)]
pub enum Font {
    PixelFont(pixelfont::PixelFont),
    DynamicFont(dynamicfont::DynamicFont),
    #[cfg(feature = "systemfonts")]
    VectorFont(vectorfont::VectorFont),
}

/// Returns the size of the pre-rendered font in pixels.
pub fn pixel_size(glyphs: &i_slint_core::graphics::BitmapGlyphs) -> PhysicalLength {
    PhysicalLength::new(glyphs.pixel_size)
}

impl i_slint_core::textlayout::FontMetrics<PhysicalLength> for Font {
    fn ascent(&self) -> PhysicalLength {
        match self {
            Font::PixelFont(pixel_font) => pixel_font.ascent(),
            Font::DynamicFont(dynamic_font) => dynamic_font.ascent(),
            #[cfg(feature = "systemfonts")]
            Font::VectorFont(vector_font) => vector_font.ascent(),
        }
    }

    fn height(&self) -> PhysicalLength {
        match self {
            Font::PixelFont(pixel_font) => pixel_font.height(),
            Font::DynamicFont(dynamic_font) => dynamic_font.height(),
            #[cfg(feature = "systemfonts")]
            Font::VectorFont(vector_font) => vector_font.height(),
        }
    }

    fn descent(&self) -> PhysicalLength {
        match self {
            Font::PixelFont(pixel_font) => pixel_font.descent(),
            Font::DynamicFont(dynamic_font) => dynamic_font.descent(),
            #[cfg(feature = "systemfonts")]
            Font::VectorFont(vector_font) => vector_font.descent(),
        }
    }

    fn x_height(&self) -> PhysicalLength {
        match self {
            Font::PixelFont(pixel_font) => pixel_font.x_height(),
            Font::DynamicFont(dynamic_font) => dynamic_font.x_height(),
            #[cfg(feature = "systemfonts")]
            Font::VectorFont(vector_font) => vector_font.x_height(),
        }
    }

    fn cap_height(&self) -> PhysicalLength {
        match self {
            Font::PixelFont(pixel_font) => pixel_font.cap_height(),
            Font::DynamicFont(dynamic_font) => dynamic_font.cap_height(),
            #[cfg(feature = "systemfonts")]
            Font::VectorFont(vector_font) => vector_font.cap_height(),
        }
    }
}

pub fn match_font(
    request: &FontRequest,
    scale_factor: ScaleFactor,
    glyph_provider: Option<&Rc<dyn GlyphProvider>>,
    glyph_id_strategy: GlyphIdStrategy,
    dynamic_font_cache: &RefCell<dynamicfont::DynamicFontCache>,
) -> Font {
    let requested_weight = request
        .weight
        .and_then(|weight| weight.try_into().ok())
        .unwrap_or(/* CSS normal */ 400);

    let requested_pixel_size: PhysicalLength =
        (request.pixel_size.unwrap_or(DEFAULT_FONT_SIZE).cast() * scale_factor).cast();

    if let Some(provider) = glyph_provider {
        if provider.supports(request) {
            return dynamic_font_cache
                .borrow_mut()
                .get_or_create(provider.clone(), request, requested_pixel_size, glyph_id_strategy)
                .into();
        }
    }

    let bitmap_font = BITMAP_FONTS.with(|fonts| {
        let fonts = fonts.borrow();

        request.family.as_ref().and_then(|requested_family| {
            fonts
                .iter()
                .filter(|bitmap_font| {
                    core::str::from_utf8(bitmap_font.family_name.as_slice()).unwrap()
                        == requested_family.as_str()
                        && bitmap_font.italic == request.italic
                })
                .min_by_key(|bitmap_font| bitmap_font.weight.abs_diff(requested_weight))
                .copied()
        })
    });

    let font = match bitmap_font {
        Some(bitmap_font) => bitmap_font,
        None => {
            #[cfg(feature = "systemfonts")]
            if let Some(vectorfont) = systemfonts::match_font(request, scale_factor) {
                return vectorfont.into();
            }
            if let Some(fallback_bitmap_font) = BITMAP_FONTS.with(|fonts| {
                let fonts = fonts.borrow();
                fonts
                    .iter()
                    .cloned()
                    .filter(|bitmap_font| bitmap_font.italic == request.italic)
                    .min_by_key(|bitmap_font| bitmap_font.weight.abs_diff(requested_weight))
                    .or_else(|| fonts.first().cloned())
            }) {
                fallback_bitmap_font
            } else {
                #[cfg(feature = "systemfonts")]
                return systemfonts::fallbackfont(request, scale_factor).into();
                #[cfg(not(feature = "systemfonts"))]
                panic!(
                    "No font fallback found. The software renderer requires enabling the `EmbedForSoftwareRenderer` option when compiling slint files."
                )
            }
        }
    };

    let nearest_pixel_size = font
        .glyphs
        .partition_point(|glyphs| pixel_size(glyphs) <= requested_pixel_size)
        .saturating_sub(1);
    let matching_glyphs = &font.glyphs[nearest_pixel_size];

    let pixel_size = if font.sdf { requested_pixel_size } else { pixel_size(matching_glyphs) };

    pixelfont::PixelFont { bitmap_font: font, glyphs: matching_glyphs, pixel_size }.into()
}

pub fn text_layout_for_font<'a, Font>(
    font: &'a Font,
    font_request: &FontRequest,
    scale_factor: ScaleFactor,
) -> TextLayout<'a, Font>
where
    Font: i_slint_core::textlayout::AbstractFont
        + i_slint_core::textlayout::TextShaper<Length = PhysicalLength>,
{
    let letter_spacing =
        font_request.letter_spacing.map(|spacing| (spacing.cast() * scale_factor).cast());

    TextLayout { font, letter_spacing }
}

pub fn register_bitmap_font(font_data: &'static BitmapFont) {
    BITMAP_FONTS.with(|fonts| fonts.borrow_mut().push(font_data))
}
