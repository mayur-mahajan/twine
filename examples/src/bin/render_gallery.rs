//! `cargo xtask sim render_gallery`: the software renderer's gallery.
//!
//! Pages: fills & opacity, rounded rects / borders / outlines, gradients, shadows, masks,
//! layers & blend modes, lines, arcs, triangles & polygons, transforms (animated). Switch pages
//! with ←/→ or by clicking the left/right third of the screen; the page number, title and what
//! to look at are logged. `R` toggles a preview that goes through the software display rotation
//! (`rotate_buffer`, shown rotated by 180° so the window size stays the same).
//!
//! Every panel format works (`TWINE_SIM_FORMAT=rgb565swapped|rgb888|xrgb8888|argb8888|l8|i1`).

use twine_core::{ColorFormat, Rect, Rotation, Size};
use twine_examples::gallery::{H, PAGES, W};
use twine_hal::Key;
use twine_render::{DrawBuf, Painter, RenderCaches, RenderConfig, rotate_area, rotate_buffer};
use twine_sim::{SimConfig, SimFrame, show_framebuffer_with_input};

struct Gallery {
    page: usize,
    rotated: bool,
    caches: RenderCaches,
    scratch: Vec<u8>,
    announced: bool,
}

impl Gallery {
    fn announce(&self) {
        let p = &PAGES[self.page];
        twine_core::info!(target: "twine::render", "page {}: {}", self.page + 1, p.title);
        twine_core::info!(target: "twine::render", "  look at: {}", p.hint);
    }

    fn input(&mut self, frame: &SimFrame) {
        let mut delta = 0i32;
        for k in &frame.keys {
            match k {
                Key::Right => delta += 1,
                Key::Left => delta -= 1,
                Key::Char('r' | 'R') => {
                    self.rotated = !self.rotated;
                    twine_core::info!(target: "twine::render", "rotation preview {}", if self.rotated { "on" } else { "off" });
                }
                _ => {}
            }
        }
        if let Some(p) = frame.clicked {
            if p.x < W / 3 {
                delta -= 1;
            } else if p.x >= 2 * W / 3 {
                delta += 1;
            }
        }
        if delta != 0 {
            self.page = (self.page as i32 + delta).rem_euclid(PAGES.len() as i32) as usize;
            self.announce();
        }
    }

    fn draw(&mut self, fb: &mut [u8], format: ColorFormat, frame: &SimFrame) {
        if !self.announced {
            self.announced = true;
            self.announce();
        }
        self.input(frame);
        let area = Rect::from_xywh(0, 0, W, H);
        let bpp = usize::from(format.bpp());
        let rotate = self.rotated && bpp >= 8;
        let target: &mut [u8] = if rotate {
            self.scratch.resize(fb.len(), 0);
            &mut self.scratch
        } else {
            fb
        };
        let Ok(buf) = DrawBuf::new_packed(target, format, area) else {
            twine_core::warn!(target: "twine::render", "cannot draw into {}", format);
            return;
        };
        (PAGES[self.page].draw)(&mut Painter::new(buf, &mut self.caches), frame.index);
        if rotate {
            // Logical chunk → rotated panel area: the whole screen maps onto itself at 180°.
            let dst = rotate_area(area, Size::new(W, H), Rotation::Deg180);
            debug_assert_eq!(dst, area);
            let stride = W as usize * bpp / 8;
            if let Err(e) = rotate_buffer(
                &self.scratch,
                stride,
                fb,
                stride,
                W as usize,
                H as usize,
                Rotation::Deg180,
                bpp / 8,
            ) {
                twine_core::warn!(target: "twine::render", "rotate_buffer failed: {}", e);
            }
        }
    }
}

fn main() {
    let cfg = SimConfig::new(W as u16, H as u16)
        .title("twine render gallery")
        .scale(2);
    let mut g = Gallery {
        page: 0,
        rotated: false,
        caches: RenderCaches::new(&RenderConfig::default()),
        scratch: Vec::new(),
        announced: false,
    };
    show_framebuffer_with_input(cfg, move |fb, format, frame| g.draw(fb, format, frame));
}
