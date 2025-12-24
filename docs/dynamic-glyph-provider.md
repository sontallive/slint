# Dynamic Glyph Provider for MCU (Rust-only, Software Renderer)

## Background
Slint's MCU path uses the software renderer and currently relies on pre-rendered bitmap fonts embedded at build time.
This is practical for Latin text, but for CJK or runtime-entered text the embedded font data becomes too large.
We want to add a runtime glyph callback path similar in spirit to LVGL's on-demand glyph loading, without changing
C++ APIs or non-software renderers.

## Goals
- Support runtime text input with on-demand glyph bitmap loading.
- Provide a Rust-only API to register a glyph provider for the software renderer.
- Keep the existing text layout pipeline intact (TextLayout/TextShaper/Renderer).
- Allow two glyph-id strategies selectable at runtime:
  - Strategy 1: Dynamic glyph-id assignment (full Unicode support).
  - Strategy 2: BMP codepoint mapping (low RAM usage).
- Add caching to avoid frequent external storage reads.

## Non-Goals
- No changes to C++ APIs.
- No changes to non-software renderers (Qt, OpenGL, WGPU, etc.).
- No full shaping engine (ligatures/complex scripts remain as-is in the software renderer).
- No new font file parser on MCU (font data lives in external storage, provider handles it).

## Current Pipeline (Summary)
- Text layout uses `TextShaper` to map text to glyphs with a `glyph_id: Option<NonZeroU16>`.
- Software renderer uses `GlyphRenderer::render_glyph(glyph_id)` to fetch bitmap data.
- Bitmap fonts are pre-rendered at build time and registered via `register_bitmap_font()`.

## Proposed Architecture
Add a new dynamic font branch for the software renderer:

- New trait: `GlyphProvider` (Rust-only, no_std friendly)
- New font type: `DynamicFont` implementing `TextShaper`, `GlyphRenderer`, and `FontMetrics`
- New runtime registration: `SoftwareRenderer::set_glyph_provider()`

The software renderer selects `DynamicFont` when a provider is present and it reports `supports(request)`.
If no provider matches, the renderer falls back to the existing bitmap font path.

### Proposed Rust API (Draft)
```
trait GlyphProvider {
    fn supports(&self, request: &FontRequest) -> bool;
    fn font_metrics(&self, request: &FontRequest, px_size: u16) -> ProviderFontMetrics;
    fn glyph_id_for_char(
        &self,
        request: &FontRequest,
        ch: char,
        strategy: GlyphIdStrategy,
    ) -> Option<NonZeroU16>;
    fn render_glyph(
        &self,
        request: &FontRequest,
        glyph_id: NonZeroU16,
        px_size: u16,
    ) -> ProviderGlyphBitmap;
}
```

Where:
- `ProviderGlyphBitmap` contains size, bearing, advance, pixel_stride, and alpha map (8-bit alpha or SDF).
- `GlyphIdStrategy` is runtime-selectable (see below).

### Glyph-ID Strategies

#### Strategy 1: Dynamic Assignment (default)
- Maintain a per-font map `char -> glyph_id` and a reverse `glyph_id -> char`.
- Allocate `glyph_id` sequentially (1..=65535) as new characters appear.
- Benefits:
  - Supports full Unicode (including non-BMP and emoji).
  - Decouples `glyph_id` from Unicode codepoint values.
- Costs:
  - The map is permanent during runtime (cannot evict IDs without breaking text caches).
  - Upper bound of 65535 distinct characters.

#### Strategy 2: BMP Codepoint Mapping (optional)
- Use Unicode codepoint directly as `glyph_id` if it fits in `NonZeroU16`.
- Benefits:
  - Minimal RAM usage (no `char -> id` map).
  - Simple implementation and stable IDs.
- Costs:
  - Only supports BMP (U+0001..U+FFFF).
  - Non-BMP characters must be mapped to missing-glyph.

### Caching Strategy
Use two separate caches per font request:

1) Glyph ID Map (Strategy 1 only)
- `HashMap<char, NonZeroU16>` and `Vec<char>` for reverse lookup.
- Not evicted to preserve glyph_id stability.

2) Bitmap LRU Cache (all strategies)
- Key: `(font_key, px_size, glyph_id)`
- Value: `ProviderGlyphBitmap`
- Eviction policy: LRU with configurable memory cap (bytes or entries).
- Safe to evict because glyph_id -> char remains stable (for Strategy 1).

### Font Keying
A `FontKey` should include:
- Family name
- Weight
- Italic
- (Optional) extra provider-specific parameters

## Integration Points
- `internal/renderers/software/fonts.rs`
  - Add `DynamicFont` to `Font` enum.
  - Update `match_font()` to query `GlyphProvider`.
- `internal/renderers/software/fonts/` (new file)
  - `dynamicfont.rs` implementing `TextShaper`, `GlyphRenderer`, `FontMetrics`.
- `internal/renderers/software/lib.rs`
  - Store `Option<Rc<dyn GlyphProvider>>` in `SoftwareRenderer`.
  - Add `set_glyph_provider()`.
- `internal/renderers/software/minimal_software_window.rs`
  - Optional forwarding helper for MCU users.

## Expected Behavior
- If provider is registered and supports the requested font, software renderer uses `DynamicFont`.
- Missing glyphs return `None`, so rendering falls back to empty glyph (advance only).
- LRU cache reduces external storage reads.

## Risks and Limitations
- `glyph_id` is limited to `NonZeroU16` (max 65535 IDs).
- Strategy 1 requires persistent char->id mapping; memory grows with distinct characters.
- No complex script shaping (consistent with current software renderer behavior).
- Cache size must be carefully chosen to fit MCU RAM limits.

## Milestones

### Milestone 1: API and Plumbing (No Rendering Change)
- Add `GlyphProvider` trait and `GlyphIdStrategy` enum (Rust-only).
- Add storage for provider in `SoftwareRenderer` and a setter API.
- Add `DynamicFont` skeleton that compiles but is not used by default.
- Acceptance:
  - Build passes, no runtime behavior change when provider is not set.

### Milestone 2: Strategy 2 (BMP Codepoint Mapping)
- Implement `DynamicFont` with BMP-only glyph_id mapping.
- Implement `render_glyph()` path to call provider and draw via software renderer.
- Add bitmap LRU cache with a small default size and configurable cap.
- Acceptance:
  - Can render BMP-only text with runtime provider and no embedded font.

### Milestone 3: Strategy 1 (Dynamic Assignment)
- Add char->id mapping and reverse lookup.
- Implement dynamic glyph-id allocation and stable mapping.
- Ensure LRU eviction does not invalidate glyph-id mapping.
- Acceptance:
  - Can render non-BMP characters via provider (if provider supports them).

### Milestone 4: Configuration and Documentation
- Add user documentation for dynamic provider usage (MCU-focused).
- Document strategy choice trade-offs and RAM sizing guidance.
- (Optional) Build configuration to skip embed font pass for MCU builds.
- Acceptance:
  - Documentation exists and examples compile.

### Milestone 5: Validation and Stress Tests
- Stress-test with large dynamic input sets and limited cache size.
- Validate glyph-id stability and cache eviction behavior.
- Acceptance:
  - No incorrect glyph rendering after cache evictions.

## Milestone Status
- [x] Milestone 1: API and plumbing
- [x] Milestone 2: Strategy 2 (BMP codepoint mapping)
- [x] Milestone 3: Strategy 1 (dynamic assignment)
- [x] Milestone 4: Configuration and documentation
- [ ] Milestone 5: Validation and stress tests

## Notes for Implementers
- Do not change `glyph_id` type unless you are ready for a wider refactor across text layout.
- Avoid locking or blocking calls inside `render_glyph()` for smooth rendering.
- Consider storing provider results in a compact format (8-bit alpha or SDF) aligned with existing renderer paths.
