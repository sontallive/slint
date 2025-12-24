// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use i_slint_core::graphics::FontRequest;

use crate::GlyphAlphaMap;

/// Selects how glyph IDs are assigned for a dynamic glyph provider.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum GlyphIdStrategy {
    /// Assign glyph IDs dynamically and sequentially.
    DynamicAssignment,
    /// Use BMP codepoint values as glyph IDs when possible.
    BmpCodepoint,
}

impl Default for GlyphIdStrategy {
    fn default() -> Self {
        Self::DynamicAssignment
    }
}

/// Font metrics returned by a glyph provider, in pixel units.
#[derive(Clone, Debug)]
pub struct ProviderFontMetrics {
    /// Font ascent in pixels (typically positive).
    pub ascent: i16,
    /// Font descent in pixels (typically negative).
    pub descent: i16,
    /// X-height in pixels.
    pub x_height: i16,
    /// Cap height in pixels.
    pub cap_height: i16,
}

/// A glyph bitmap and its metrics returned by a glyph provider, in pixel units.
#[derive(Clone, Debug)]
pub struct ProviderGlyphBitmap {
    /// Horizontal bearing relative to the baseline in pixels.
    pub x: i16,
    /// Vertical bearing relative to the baseline in pixels.
    pub y: i16,
    /// Glyph bitmap width in pixels.
    pub width: i16,
    /// Glyph bitmap height in pixels.
    pub height: i16,
    /// Horizontal advance in pixels.
    pub advance: i16,
    /// Pixel stride in bytes for each row of the alpha map.
    pub pixel_stride: u16,
    /// Alpha map or SDF data for the glyph.
    pub alpha_map: GlyphAlphaMap,
    /// True if the glyph uses signed distance field data.
    pub sdf: bool,
}

impl ProviderGlyphBitmap {
    /// Returns true if there is no glyph bitmap data to render.
    pub fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0 || self.alpha_map.is_empty()
    }
}

/// Supplies glyph data at runtime for the software renderer.
pub trait GlyphProvider {
    /// Returns true if this provider can satisfy the requested font.
    fn supports(&self, request: &FontRequest) -> bool;
    /// Returns font metrics in pixels for the requested font and size.
    fn font_metrics(&self, request: &FontRequest, px_size: u16) -> ProviderFontMetrics;
    /// Returns a glyph ID for the given character and strategy.
    fn glyph_id_for_char(
        &self,
        request: &FontRequest,
        ch: char,
        strategy: GlyphIdStrategy,
    ) -> Option<core::num::NonZeroU16>;
    /// Renders a glyph bitmap for the given ID and size.
    fn render_glyph(
        &self,
        request: &FontRequest,
        glyph_id: core::num::NonZeroU16,
        px_size: u16,
    ) -> ProviderGlyphBitmap;
}
