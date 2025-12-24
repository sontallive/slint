// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: MIT

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::num::NonZeroU16;
use std::rc::Rc;
use std::time::Instant;

use slint::platform::software_renderer::{
    FontRequest, GlyphAlphaMap, GlyphIdStrategy, GlyphProvider, MinimalSoftwareWindow,
    PremultipliedRgbaColor, ProviderFontMetrics, ProviderGlyphBitmap, RepaintBufferType,
};
use slint::platform::{Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent};
use slint::LogicalPosition;
use softbuffer::Surface;
use std::error::Error;
use std::num::NonZeroU32;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize as WinitPhysicalSize;
use winit::event::{ElementState, MouseButton, StartCause, WindowEvent as WinitWindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

slint::include_modules!();

const DISPLAY_WIDTH: u32 = 420;
const DISPLAY_HEIGHT: u32 = 300;

#[derive(Default)]
struct ProviderState {
    next_dynamic_id: u16,
    char_to_id: BTreeMap<char, NonZeroU16>,
    id_to_codepoint: BTreeMap<u16, u32>,
    glyph_id_calls: usize,
    render_calls: usize,
}

struct FontdueGlyphProvider {
    font: Rc<fontdue::Font>,
    state: RefCell<ProviderState>,
}

impl FontdueGlyphProvider {
    fn new(font: Rc<fontdue::Font>) -> Self {
        Self { font, state: RefCell::new(ProviderState { next_dynamic_id: 1, ..Default::default() }) }
    }

    fn allocate_id(&self, codepoint: u32) -> Option<NonZeroU16> {
        let mut state = self.state.borrow_mut();
        let next = state.next_dynamic_id;
        if next == 0 {
            return None;
        }
        let id = NonZeroU16::new(next)?;
        state.next_dynamic_id = if next == u16::MAX { 0 } else { next + 1 };
        state.id_to_codepoint.insert(id.get(), codepoint);
        Some(id)
    }

    fn codepoint_for_glyph_id(&self, glyph_id: NonZeroU16) -> u32 {
        self.state
            .borrow()
            .id_to_codepoint
            .get(&glyph_id.get())
            .copied()
            .unwrap_or(glyph_id.get() as u32)
    }
}

impl GlyphProvider for FontdueGlyphProvider {
    fn supports(&self, _request: &FontRequest) -> bool {
        true
    }

    fn font_metrics(&self, _request: &FontRequest, px_size: u16) -> ProviderFontMetrics {
        let px = px_size.max(1) as f32;
        let Some(line) = self.font.horizontal_line_metrics(px) else {
            let px_i16 = px_size.max(1) as i16;
            return ProviderFontMetrics {
                ascent: (px_i16 * 3) / 4,
                descent: -((px_i16 * 1) / 4),
                x_height: (px_i16 * 2) / 4,
                cap_height: (px_i16 * 3) / 4,
            };
        };

        let ascent = line.ascent.round() as i16;
        let descent = line.descent.round() as i16;

        let x_height = self.font.rasterize('x', px).0.height as i16;
        let cap_height = self.font.rasterize('H', px).0.height as i16;

        ProviderFontMetrics { ascent, descent, x_height, cap_height }
    }

    fn glyph_id_for_char(
        &self,
        _request: &FontRequest,
        ch: char,
        strategy: GlyphIdStrategy,
    ) -> Option<NonZeroU16> {
        let mut state = self.state.borrow_mut();
        state.glyph_id_calls += 1;
        drop(state);

        match strategy {
            GlyphIdStrategy::BmpCodepoint => {
                let codepoint = ch as u32;
                if codepoint == 0 || codepoint > u16::MAX as u32 {
                    None
                } else {
                    NonZeroU16::new(codepoint as u16)
                }
            }
            GlyphIdStrategy::DynamicAssignment => {
                if let Some(existing) = self.state.borrow().char_to_id.get(&ch).copied() {
                    // 缓存命中，不打印日志以减少输出
                    return Some(existing);
                }

                let codepoint = ch as u32;

                // 检查字体是否支持这个字符
                let glyph_index = self.font.lookup_glyph_index(ch);
                if glyph_index == 0 {
                    println!("⚠️  字体不支持: '{}' (U+{:04X}) - 将显示方框", ch, codepoint);
                }

                let id = self.allocate_id(codepoint)?;

                let mut state = self.state.borrow_mut();
                state.char_to_id.insert(ch, id);
                println!("✨ 新字符分配ID: '{}' (U+{:04X}) -> ID {} [字体glyph: {}]",
                    ch, codepoint, id, glyph_index);
                Some(id)
            }
        }
    }

    fn render_glyph(
        &self,
        _request: &FontRequest,
        glyph_id: NonZeroU16,
        px_size: u16,
    ) -> ProviderGlyphBitmap {
        let render_count = {
            let mut state = self.state.borrow_mut();
            state.render_calls += 1;
            state.render_calls
        };

        let codepoint = self.codepoint_for_glyph_id(glyph_id);
        let px = px_size.max(1) as f32;
        let ch = char::from_u32(codepoint).unwrap_or('\u{FFFD}');

        let (metrics, data) = self.font.rasterize(ch, px);

        println!("🎨 渲染 #{}: '{}' ({}px) - 尺寸: {}x{}, 数据: {} bytes",
            render_count, ch, px_size, metrics.width, metrics.height, data.len());

        let width = metrics.width as i16;
        let height = metrics.height as i16;
        let stride = metrics.width as u16;
        let advance = metrics.advance_width.round() as i16;

        ProviderGlyphBitmap {
            x: metrics.xmin as i16,
            y: metrics.ymin as i16,
            width,
            height,
            advance,
            pixel_stride: stride,
            alpha_map: GlyphAlphaMap::Shared(Rc::<[u8]>::from(data)),
            sdf: false,
        }
    }
}

struct CustomPlatform {
    window: Rc<MinimalSoftwareWindow>,
    start_time: Instant,
}

impl CustomPlatform {
    fn new() -> Self {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        window.set_size(slint::WindowSize::Physical(slint::PhysicalSize::new(
            DISPLAY_WIDTH,
            DISPLAY_HEIGHT,
        )));

        Self {
            window,
            start_time: Instant::now(),
        }
    }

    fn window(&self) -> Rc<MinimalSoftwareWindow> {
        self.window.clone()
    }
}

impl Platform for CustomPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }
}

struct AppState {
    window: Option<Rc<Window>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    slint_window: Rc<MinimalSoftwareWindow>,
    buffer: Vec<PremultipliedRgbaColor>,
    ui: Option<DynamicGlyphTest>,
    cursor_position: LogicalPosition,
    pending_redraw: bool,
}

impl AppState {
    fn new(slint_window: Rc<MinimalSoftwareWindow>) -> Self {
        let size = (DISPLAY_WIDTH * DISPLAY_HEIGHT) as usize;
        Self {
            window: None,
            surface: None,
            slint_window,
            buffer: vec![PremultipliedRgbaColor::default(); size],
            ui: None,
            cursor_position: LogicalPosition::new(0.0, 0.0),
            pending_redraw: false,
        }
    }

    fn render_and_present(&mut self) {
        let pixel_stride = DISPLAY_WIDTH as usize;

        self.slint_window.draw_if_needed(|renderer| {
            renderer.render(&mut self.buffer, pixel_stride);
        });

        if let Some(surface) = &mut self.surface {
            let mut surface_buffer = surface.buffer_mut().unwrap();

            for (i, pixel) in self.buffer.iter().enumerate() {
                surface_buffer[i] = ((pixel.red as u32) << 16)
                    | ((pixel.green as u32) << 8)
                    | (pixel.blue as u32);
            }

            surface_buffer.present().unwrap();
        }
    }
}

impl ApplicationHandler for AppState {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _cause: StartCause) {
        slint::platform::update_timers_and_animations();

        if self.pending_redraw {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            self.pending_redraw = false;
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attrs = Window::default_attributes()
            .with_title("Dynamic Glyph Provider - Desktop Demo")
            .with_inner_size(WinitPhysicalSize::new(DISPLAY_WIDTH, DISPLAY_HEIGHT))
            .with_resizable(false);

        let window = Rc::new(event_loop.create_window(window_attrs).unwrap());

        let context = softbuffer::Context::new(window.clone()).unwrap();
        let mut surface = Surface::new(&context, window.clone()).unwrap();

        surface
            .resize(
                NonZeroU32::new(DISPLAY_WIDTH).unwrap(),
                NonZeroU32::new(DISPLAY_HEIGHT).unwrap(),
            )
            .unwrap();

        self.window = Some(window.clone());
        self.surface = Some(surface);

        self.slint_window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });

        let ui = DynamicGlyphTest::new().unwrap();
        ui.set_input_text("Hello, 你好, 🙂".into());
        ui.set_strategy_label("DynamicAssignment".into());

        ui.show().unwrap();
        self.ui = Some(ui);

        window.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WinitWindowEvent,
    ) {
        match event {
            WinitWindowEvent::CloseRequested => {
                event_loop.exit();
            }

            WinitWindowEvent::RedrawRequested => {
                self.render_and_present();
            }

            WinitWindowEvent::CursorMoved { position, .. } => {
                let scale_factor = self.slint_window.scale_factor();
                self.cursor_position = LogicalPosition::new(
                    position.x as f32 / scale_factor,
                    position.y as f32 / scale_factor,
                );

                self.slint_window.dispatch_event(WindowEvent::PointerMoved {
                    position: self.cursor_position,
                });
                self.pending_redraw = true;
            }

            WinitWindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    MouseButton::Left => PointerEventButton::Left,
                    MouseButton::Right => PointerEventButton::Right,
                    MouseButton::Middle => PointerEventButton::Middle,
                    _ => PointerEventButton::Other,
                };

                let event = match state {
                    ElementState::Pressed => WindowEvent::PointerPressed {
                        position: self.cursor_position,
                        button,
                    },
                    ElementState::Released => WindowEvent::PointerReleased {
                        position: self.cursor_position,
                        button,
                    },
                };

                self.slint_window.dispatch_event(event);
                self.pending_redraw = true;
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.slint_window.has_active_animations() && let Some(window) = &self.window {
            window.request_redraw();
        }

        if let Some(duration) = slint::platform::duration_until_next_timer_update() {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                std::time::Instant::now() + duration,
            ));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    println!("\n🚀 启动 Dynamic Glyph Provider 桌面演示程序");
    println!("═══════════════════════════════════════════\n");

    // 字体选项:
    // LXGWWenKai - 支持中文和常用符号
    // 注意：彩色 emoji 字体（如 Apple Color Emoji）使用位图格式，
    // fontdue 无法提取轮廓数据，只能使用包含轮廓的字体

    let font_path = "/Users/sontal/code/reader-slint/ui/font/LXGWWenKai-Regular.ttf";

    println!("📚 加载字体: {}", font_path);
    let font_bytes = std::fs::read(font_path)?;
    let font = Rc::new(fontdue::Font::from_bytes(font_bytes, fontdue::FontSettings::default())?);
    let provider = Rc::new(FontdueGlyphProvider::new(font));
    println!("✅ 字体加载成功");

    let platform = CustomPlatform::new();
    let slint_window = platform.window();

    slint_window.set_glyph_provider(Some(provider.clone()));
    slint_window.set_glyph_id_strategy(GlyphIdStrategy::DynamicAssignment);
    slint_window.set_glyph_cache_bytes(64 * 1024);
    println!("⚙️  字形策略: DynamicAssignment");
    println!("💾 缓存大小: 64 KB");
    println!("\n开始监听字形读取事件...\n");

    slint::platform::set_platform(Box::new(platform))?;

    let event_loop = EventLoop::new()?;
    let mut app_state = AppState::new(slint_window);

    event_loop.run_app(&mut app_state)?;

    Ok(())
}
