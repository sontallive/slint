// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

use i_slint_core::graphics::FontRequest;
use i_slint_core::lengths::ScaleFactor;

use crate::{GlyphIdStrategy, GlyphProvider};

use super::{DynamicFontCache, Font};

/// Manages dynamic glyph provider state and font selection for the software renderer.
pub(crate) struct FontService {
    glyph_provider: RefCell<Option<Rc<dyn GlyphProvider>>>,
    glyph_id_strategy: Cell<GlyphIdStrategy>,
    dynamic_font_cache: RefCell<DynamicFontCache>,
}

impl Default for FontService {
    fn default() -> Self {
        Self {
            glyph_provider: Default::default(),
            glyph_id_strategy: Cell::new(GlyphIdStrategy::default()),
            dynamic_font_cache: Default::default(),
        }
    }
}

impl FontService {
    pub(crate) fn set_glyph_provider(&self, provider: Option<Rc<dyn GlyphProvider>>) {
        *self.glyph_provider.borrow_mut() = provider;
        self.dynamic_font_cache.borrow_mut().clear();
    }

    pub(crate) fn set_glyph_id_strategy(&self, strategy: GlyphIdStrategy) {
        if self.glyph_id_strategy.replace(strategy) != strategy {
            self.dynamic_font_cache.borrow_mut().clear();
        }
    }

    pub(crate) fn set_glyph_cache_bytes(&self, bytes: usize) {
        self.dynamic_font_cache.borrow_mut().set_glyph_cache_bytes(bytes);
    }

    pub(crate) fn match_font(&self, request: &FontRequest, scale_factor: ScaleFactor) -> Font {
        let glyph_provider = self.glyph_provider.borrow();
        super::match_font(
            request,
            scale_factor,
            glyph_provider.as_ref(),
            self.glyph_id_strategy.get(),
            &self.dynamic_font_cache,
        )
    }
}
