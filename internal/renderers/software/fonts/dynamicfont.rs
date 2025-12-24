// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::num::NonZeroU16;

use i_slint_core::SharedString;
use i_slint_core::graphics::FontRequest;
use i_slint_core::textlayout::{FontMetrics, Glyph, TextShaper};

use super::{GlyphRenderer, RenderableGlyph};
use crate::PhysicalLength;
use crate::fixed::Fixed;
use crate::{GlyphIdStrategy, GlyphProvider, ProviderFontMetrics, ProviderGlyphBitmap};

const DEFAULT_GLYPH_CACHE_BYTES: usize = 64 * 1024;

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
struct DynamicFontKey {
    family: Option<SharedString>,
    weight: Option<i32>,
    italic: bool,
    pixel_size: i16,
}

impl DynamicFontKey {
    fn new(request: &FontRequest, pixel_size: PhysicalLength) -> Self {
        Self {
            family: request.family.clone(),
            weight: request.weight,
            italic: request.italic,
            pixel_size: pixel_size.get(),
        }
    }
}

pub(crate) struct DynamicFontCache {
    entries: BTreeMap<DynamicFontKey, Rc<DynamicFontData>>,
    glyph_cache_bytes: usize,
}

impl Default for DynamicFontCache {
    fn default() -> Self {
        Self { entries: BTreeMap::new(), glyph_cache_bytes: DEFAULT_GLYPH_CACHE_BYTES }
    }
}

impl DynamicFontCache {
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn set_glyph_cache_bytes(&mut self, glyph_cache_bytes: usize) {
        self.glyph_cache_bytes = glyph_cache_bytes;
        self.entries.clear();
    }

    pub(crate) fn get_or_create(
        &mut self,
        provider: Rc<dyn GlyphProvider>,
        request: &FontRequest,
        pixel_size: PhysicalLength,
        strategy: GlyphIdStrategy,
    ) -> DynamicFont {
        let key = DynamicFontKey::new(request, pixel_size);
        if let Some(existing) = self.entries.get(&key) {
            return DynamicFont { data: existing.clone() };
        }

        let px_size = pixel_size_to_u16(pixel_size);
        let metrics = provider.font_metrics(request, px_size);
        let data = Rc::new(DynamicFontData {
            request: request.clone(),
            pixel_size,
            provider,
            strategy,
            metrics,
            glyph_cache: RefCell::new(GlyphBitmapCache::new(self.glyph_cache_bytes)),
        });
        self.entries.insert(key, data.clone());
        DynamicFont { data }
    }
}

pub struct DynamicFont {
    data: Rc<DynamicFontData>,
}

struct DynamicFontData {
    request: FontRequest,
    pixel_size: PhysicalLength,
    provider: Rc<dyn GlyphProvider>,
    strategy: GlyphIdStrategy,
    metrics: ProviderFontMetrics,
    glyph_cache: RefCell<GlyphBitmapCache>,
}

impl DynamicFontData {
    fn pixel_size_u16(&self) -> u16 {
        pixel_size_to_u16(self.pixel_size)
    }

    fn glyph_id_for_char(&self, ch: char) -> Option<NonZeroU16> {
        match self.strategy {
            GlyphIdStrategy::BmpCodepoint => bmp_codepoint_glyph_id(ch),
            GlyphIdStrategy::DynamicAssignment => self
                .provider
                .glyph_id_for_char(&self.request, ch, self.strategy),
        }
    }
}

struct GlyphBitmapCache {
    entries: Vec<GlyphCacheEntry>,
    current_bytes: usize,
    max_bytes: usize,
}

struct GlyphCacheEntry {
    glyph_id: NonZeroU16,
    bitmap: ProviderGlyphBitmap,
    weight: usize,
}

impl GlyphBitmapCache {
    fn new(max_bytes: usize) -> Self {
        Self { entries: Vec::new(), current_bytes: 0, max_bytes }
    }

    fn get_or_insert_with(
        &mut self,
        glyph_id: NonZeroU16,
        render: impl FnOnce() -> ProviderGlyphBitmap,
    ) -> ProviderGlyphBitmap {
        if let Some(existing_index) =
            self.entries.iter().position(|entry| entry.glyph_id == glyph_id)
        {
            let entry = self.entries.remove(existing_index);
            self.entries.push(entry);
            return self.entries.last().unwrap().bitmap.clone();
        }

        let bitmap = render();
        if self.max_bytes == 0 {
            return bitmap;
        }

        let weight = bitmap.alpha_map.len();
        self.entries.push(GlyphCacheEntry { glyph_id, bitmap: bitmap.clone(), weight });
        self.current_bytes = self.current_bytes.saturating_add(weight);

        while self.current_bytes > self.max_bytes && self.entries.len() > 1 {
            if let Some(entry) = self.entries.first() {
                self.current_bytes = self.current_bytes.saturating_sub(entry.weight);
            }
            self.entries.remove(0);
        }

        bitmap
    }
}

impl DynamicFont {
    fn glyph_bitmap(&self, glyph_id: NonZeroU16) -> ProviderGlyphBitmap {
        let mut cache = self.data.glyph_cache.borrow_mut();
        cache.get_or_insert_with(glyph_id, || {
            self.data.provider.render_glyph(
                &self.data.request,
                glyph_id,
                self.data.pixel_size_u16(),
            )
        })
    }

    fn default_advance(&self) -> PhysicalLength {
        self.data.pixel_size
    }
}

impl TextShaper for DynamicFont {
    type LengthPrimitive = i16;
    type Length = PhysicalLength;

    fn shape_text<GlyphStorage: core::iter::Extend<Glyph<PhysicalLength>>>(
        &self,
        text: &str,
        glyphs: &mut GlyphStorage,
    ) {
        glyphs.extend(text.char_indices().map(|(byte_offset, ch)| {
            let glyph_id = self.data.glyph_id_for_char(ch);

            let advance = glyph_id
                .map(|glyph_id| self.glyph_bitmap(glyph_id).advance)
                .map(PhysicalLength::new)
                .unwrap_or_else(|| self.default_advance());

            Glyph { glyph_id, advance, text_byte_offset: byte_offset, ..Default::default() }
        }));
    }

    fn glyph_for_char(&self, ch: char) -> Option<Glyph<PhysicalLength>> {
        let glyph_id = self.data.glyph_id_for_char(ch)?;
        let advance = PhysicalLength::new(self.glyph_bitmap(glyph_id).advance);

        Some(Glyph {
            glyph_id: Some(glyph_id),
            advance,
            text_byte_offset: 0,
            ..Default::default()
        })
    }

    fn max_lines(&self, max_height: PhysicalLength) -> usize {
        (max_height / self.height()).get() as _
    }
}

impl i_slint_core::textlayout::FontMetrics<PhysicalLength> for DynamicFont {
    fn ascent(&self) -> PhysicalLength {
        PhysicalLength::new(self.data.metrics.ascent)
    }

    fn height(&self) -> PhysicalLength {
        PhysicalLength::new(self.data.metrics.ascent - self.data.metrics.descent)
    }

    fn descent(&self) -> PhysicalLength {
        PhysicalLength::new(self.data.metrics.descent)
    }

    fn x_height(&self) -> PhysicalLength {
        PhysicalLength::new(self.data.metrics.x_height)
    }

    fn cap_height(&self) -> PhysicalLength {
        PhysicalLength::new(self.data.metrics.cap_height)
    }
}

impl GlyphRenderer for DynamicFont {
    fn render_glyph(&self, glyph_id: NonZeroU16) -> Option<RenderableGlyph> {
        let glyph = self.glyph_bitmap(glyph_id);
        if glyph.is_empty() {
            return None;
        }

        let width = glyph.width.max(0);
        let height = glyph.height.max(0);
        if width == 0 || height == 0 {
            return None;
        }

        Some(RenderableGlyph {
            x: Fixed::from_integer(glyph.x as i32),
            y: Fixed::from_integer(glyph.y as i32),
            width: PhysicalLength::new(width),
            height: PhysicalLength::new(height),
            alpha_map: glyph.alpha_map,
            pixel_stride: glyph.pixel_stride,
            sdf: glyph.sdf,
        })
    }

    fn scale_delta(&self) -> Fixed<u16, 8> {
        Fixed::from_integer(1)
    }
}

fn pixel_size_to_u16(pixel_size: PhysicalLength) -> u16 {
    let px = pixel_size.get();
    if px <= 0 {
        0
    } else {
        px as u16
    }
}

fn bmp_codepoint_glyph_id(ch: char) -> Option<NonZeroU16> {
    let codepoint = ch as u32;
    if codepoint == 0 || codepoint > u16::MAX as u32 {
        None
    } else {
        NonZeroU16::new(codepoint as u16)
    }
}
