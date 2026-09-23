//! The simulator window: winit 0.30 event loop + softbuffer presentation.
//!
//! The panel image is converted to `0x00RRGGBB` and scaled nearest-neighbour to the window.
//! Presentation happens only when the panel changed; between frames the event loop sleeps with
//! `ControlFlow::WaitUntil(next deadline)`, so an idle simulator uses no CPU (P1).

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Instant as StdInstant;

use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::Key as WKey;
use winit::window::{Window, WindowId};

use crate::app::{SimApp, SimError};
use crate::hotkeys::Hotkey;
use crate::input::{WheelAccum, map_key, mouse_to_panel};

struct Gfx {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    // Kept alive for the surface.
    _context: Context<Rc<Window>>,
}

struct Runner {
    app: SimApp,
    gfx: Option<Gfx>,
    error: Option<SimError>,
    next_frame: StdInstant,
    shift: bool,
    cursor: (f64, f64),
    wheel: WheelAccum,
    xrgb: Vec<u32>,
    needs_present: bool,
}

/// Opens the window and runs the event loop until the window is closed.
pub(crate) fn run(app: SimApp) -> Result<(), SimError> {
    let event_loop = EventLoop::new().map_err(|e| SimError::Window(e.to_string()))?;
    log::info!(
        target: "twine::sim",
        "window {}x{} {} scale {}{} — press F1 for hotkeys",
        app.cfg.width,
        app.cfg.height,
        app.cfg.format,
        app.cfg.scale,
        app.cfg.bus_hz.map_or_else(String::new, |hz| format!(", bus {hz} Hz"))
    );
    let mut runner = Runner {
        app,
        gfx: None,
        error: None,
        next_frame: StdInstant::now(),
        shift: false,
        cursor: (0.0, 0.0),
        wheel: WheelAccum::default(),
        xrgb: Vec::new(),
        needs_present: true,
    };
    event_loop
        .run_app(&mut runner)
        .map_err(|e| SimError::Window(e.to_string()))?;
    match runner.error.take() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

impl Runner {
    fn create_window(&mut self, el: &ActiveEventLoop) -> Result<Gfx, SimError> {
        let cfg = &self.app.cfg;
        let size = PhysicalSize::new(
            u32::from(cfg.width) * u32::from(cfg.scale),
            u32::from(cfg.height) * u32::from(cfg.scale),
        );
        let attrs = Window::default_attributes()
            .with_title(cfg.title.clone())
            .with_inner_size(size)
            .with_resizable(false);
        let window = Rc::new(
            el.create_window(attrs)
                .map_err(|e| SimError::Window(e.to_string()))?,
        );
        let context = Context::new(window.clone()).map_err(|e| SimError::Window(e.to_string()))?;
        let surface = Surface::new(&context, window.clone()).map_err(|e| SimError::Window(e.to_string()))?;
        Ok(Gfx {
            window,
            surface,
            _context: context,
        })
    }

    /// Window pixels per panel pixel (the actual ratio, normally the configured scale).
    fn scale(&self) -> f64 {
        self.gfx.as_ref().map_or(f64::from(self.app.cfg.scale), |g| {
            f64::from(g.window.inner_size().width) / f64::from(self.app.cfg.width.max(1))
        })
    }

    fn panel_point(&self) -> twine_core::Point {
        mouse_to_panel(
            self.cursor,
            self.scale(),
            (self.app.cfg.width, self.app.cfg.height),
        )
    }

    /// Advances the app: reclaims the draw buffer, renders a frame when due, and requests a redraw
    /// when the panel changed. Returns the next wake-up deadline (`None` = poll continuously).
    fn tick(&mut self) -> Option<StdInstant> {
        let now = StdInstant::now();
        self.app.reclaim();
        let interval = self.app.frame_interval();
        let due = interval.is_none_or(|_| now >= self.next_frame);
        if self.app.has_buffer() && due {
            self.app.render_frame();
            if let Some(i) = interval {
                self.next_frame += i;
                if self.next_frame < now {
                    self.next_frame = now + i;
                }
            }
            self.app.reclaim();
        }
        if self.app.display.take_dirty().is_some() {
            self.needs_present = true;
        }
        if self.needs_present {
            if let Some(g) = &self.gfx {
                g.window.request_redraw();
            }
        }
        if self.app.has_buffer() {
            interval.map(|_| self.next_frame)
        } else {
            Some(self.app.display.busy_until().unwrap_or(now))
        }
    }

    fn present(&mut self) -> Result<(), String> {
        let Some(g) = &mut self.gfx else {
            return Ok(());
        };
        let size = g.window.inner_size();
        let (Some(ww), Some(wh)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return Ok(());
        };
        g.surface.resize(ww, wh).map_err(|e| e.to_string())?;
        self.app.display.panel_xrgb_into(&mut self.xrgb);
        let (pw, ph) = (usize::from(self.app.cfg.width), usize::from(self.app.cfg.height));
        let (ww, wh) = (size.width as usize, size.height as usize);
        let mut buffer = g.surface.buffer_mut().map_err(|e| e.to_string())?;
        if pw > 0 && ph > 0 {
            let xmap: Vec<usize> = (0..ww).map(|x| x * pw / ww).collect();
            for (y, row) in buffer.chunks_exact_mut(ww).enumerate().take(wh) {
                let src = &self.xrgb[(y * ph / wh) * pw..][..pw];
                for (dst, &sx) in row.iter_mut().zip(&xmap) {
                    *dst = src[sx];
                }
            }
        }
        buffer.present().map_err(|e| e.to_string())?;
        self.needs_present = false;
        self.app.on_presented();
        Ok(())
    }

    fn keyboard(&mut self, event: &winit::event::KeyEvent) {
        let pressed = event.state == ElementState::Pressed;
        if let WKey::Named(named) = &event.logical_key {
            if let Some(hk) = Hotkey::from_winit(*named) {
                if pressed && !event.repeat {
                    self.app.hotkey(hk);
                }
                return; // hotkeys are consumed
            }
        }
        if event.repeat {
            return; // the engine generates its own long-press repeats
        }
        if let Some(k) = map_key(&event.logical_key, event.text.as_deref(), self.shift) {
            self.app.devices().state.borrow_mut().key(k, pressed);
        }
    }
}

impl ApplicationHandler for Runner {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        match self.create_window(el) {
            Ok(g) => {
                g.window.request_redraw();
                self.gfx = Some(g);
            }
            Err(e) => {
                self.error = Some(e);
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(_) => {
                self.needs_present = true;
                if let Some(g) = &self.gfx {
                    g.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.present() {
                    self.error = Some(SimError::Window(e));
                    el.exit();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x, position.y);
                let p = self.panel_point();
                self.app.devices().state.borrow_mut().pointer_move(p);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => {
                        let p = self.panel_point();
                        let mut s = self.app.devices().state.borrow_mut();
                        if pressed {
                            s.pointer_press(p);
                        } else {
                            s.pointer_release();
                        }
                    }
                    MouseButton::Middle => {
                        self.app.devices().state.borrow_mut().encoder_button(pressed);
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let diff = match delta {
                    MouseScrollDelta::LineDelta(_, y) => self.wheel.lines(f64::from(y)),
                    MouseScrollDelta::PixelDelta(p) => self.wheel.pixels(p.y),
                };
                self.app.devices().state.borrow_mut().encoder_rotate(diff);
            }
            WindowEvent::ModifiersChanged(m) => self.shift = m.state().shift_key(),
            WindowEvent::KeyboardInput {
                event, is_synthetic, ..
            } => {
                if !is_synthetic {
                    self.keyboard(&event);
                }
            }
            WindowEvent::Focused(false) => {
                self.app.devices().state.borrow_mut().release_all();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        match self.tick() {
            Some(deadline) => el.set_control_flow(ControlFlow::WaitUntil(deadline)),
            None => el.set_control_flow(ControlFlow::Poll),
        }
    }
}
