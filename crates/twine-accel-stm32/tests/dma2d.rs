//! Register programming of `Dma2d`, checked against a recording mock register file.
#![allow(clippy::unreadable_literal)]

use twine_accel_stm32::bits::{
    AM_MULTIPLY, AM_NO_MODIFY, AM_REPLACE, CM_A8, CM_ARGB8888, CM_L8, CM_RGB565, CR_START, ISR_CEIF,
    ISR_TEIF, MODE_M2M, MODE_M2M_BLEND, MODE_M2M_PFC, MODE_R2M, PFCCR_START,
};
use twine_accel_stm32::mock::{Access, MockRegs};
use twine_accel_stm32::{Dma2d, Reg};
use twine_core::{Color, ColorFormat, Opa, Rect};
use twine_render::{AccelResult, DrawAccel, DrawBuf, ImagePixels};

const W: i32 = 64;
const H: i32 = 32;

/// A `W × H` buffer at screen `(10, 20)` in `format`.
fn with_buf<R>(format: ColorFormat, f: impl FnOnce(&mut DrawBuf<'_>, usize) -> R) -> R {
    let bytes = format.bpp() as usize / 8;
    let mut raw = vec![0u8; (W * H) as usize * bytes + 4];
    let mem = aligned4(&mut raw);
    let base = mem.as_ptr() as usize;
    let mut buf = DrawBuf::new_packed(mem, format, Rect::from_xywh(10, 20, W, H)).unwrap();
    f(&mut buf, base)
}

/// The part of `v` starting at a 4-byte aligned address (like a real framebuffer).
fn aligned4(v: &mut [u8]) -> &mut [u8] {
    let off = v.as_ptr().align_offset(4);
    &mut v[off..]
}

/// `NLR` for `w × h`.
fn nlr(w: u32, h: u32) -> u32 {
    (w << 16) + h
}

fn mode(dma: &Dma2d<MockRegs>) -> u32 {
    dma.regs().last_write(Reg::Cr).unwrap() >> 16
}

#[test]
fn fill_programs_r2m_registers() {
    let mut dma = Dma2d::new(MockRegs::new());
    with_buf(ColorFormat::Rgb565, |buf, base| {
        let area = Rect::from_xywh(12, 21, 40, 10);
        let r = dma.fill(buf, area, Color::hex(0x00FF00), Opa::COVER);
        assert_eq!(r, AccelResult::Queued);
        let regs = dma.regs();
        assert_eq!(regs.get(Reg::Opfccr), CM_RGB565);
        assert_eq!(regs.get(Reg::Ocolr), 0x07E0);
        let origin = base + W as usize * 2 + 2 * 2; // row 21 − 20, column 12 − 10
        assert_eq!(regs.get(Reg::Omar), origin as u32);
        assert_eq!(regs.get(Reg::Oor), (W - 40) as u32);
        assert_eq!(regs.get(Reg::Nlr), nlr(40, 10));
        assert_eq!(regs.last_write(Reg::Cr), Some((MODE_R2M << 16) | CR_START));
        // START is the last write; no foreground/background is programmed.
        assert!(matches!(regs.log().last(), Some(Access::Write(Reg::Cr, _))));
        assert_eq!(regs.last_write(Reg::Fgpfccr), None);
        assert_eq!(regs.last_write(Reg::Bgmar), None);
    });
    assert!(dma.is_busy());
    dma.wait();
    assert!(!dma.is_busy());
    assert_eq!(dma.error_count(), 0);
}

#[test]
fn fill_argb8888_and_xrgb8888_color_layout() {
    for format in [ColorFormat::Argb8888, ColorFormat::Xrgb8888] {
        let mut dma = Dma2d::new(MockRegs::new());
        with_buf(format, |buf, _| {
            dma.fill(
                buf,
                Rect::from_xywh(10, 20, W, H),
                Color::hex(0x123456),
                Opa::COVER,
            );
        });
        assert_eq!(dma.regs().get(Reg::Opfccr), CM_ARGB8888);
        assert_eq!(dma.regs().get(Reg::Ocolr), 0xFF12_3456);
        assert_eq!(dma.regs().get(Reg::Oor), 0);
    }
}

#[test]
fn fill_with_opa_uses_blend_mode() {
    let mut dma = Dma2d::new(MockRegs::new());
    with_buf(ColorFormat::Rgb565, |buf, base| {
        let area = Rect::from_xywh(10, 20, 32, 16);
        assert_eq!(
            dma.fill(buf, area, Color::hex(0x102030), Opa(128)),
            AccelResult::Queued
        );
        let regs = dma.regs();
        assert_eq!(mode(&dma), MODE_M2M_BLEND);
        // FG: fixed color through A8 with the alpha replaced by opa.
        assert_eq!(regs.get(Reg::Fgpfccr), CM_A8 | (AM_REPLACE << 16) | (128 << 24));
        assert_eq!(regs.get(Reg::Fgcolr), 0x102030);
        assert_eq!(regs.get(Reg::Fgmar), base as u32);
        assert_eq!(
            regs.get(Reg::Fgor),
            (W * 2 - 32) as u32,
            "FG lines span one destination line"
        );
        // BG = output = destination.
        assert_eq!(regs.get(Reg::Bgmar), base as u32);
        assert_eq!(regs.get(Reg::Omar), base as u32);
        assert_eq!(regs.get(Reg::Bgor), (W - 32) as u32);
        assert_eq!(regs.get(Reg::Oor), (W - 32) as u32);
        assert_eq!(regs.get(Reg::Bgpfccr), CM_RGB565);
        assert_eq!(regs.get(Reg::Opfccr), CM_RGB565);
        assert_eq!(regs.get(Reg::Nlr), nlr(32, 16));
    });
}

#[test]
fn xrgb8888_background_alpha_is_replaced() {
    let mut dma = Dma2d::new(MockRegs::new());
    with_buf(ColorFormat::Xrgb8888, |buf, _| {
        dma.fill(buf, Rect::from_xywh(10, 20, 32, 16), Color::WHITE, Opa(100));
    });
    assert_eq!(
        dma.regs().get(Reg::Bgpfccr),
        CM_ARGB8888 | (AM_REPLACE << 16) | (0xFF << 24)
    );
}

#[test]
fn blit_rgb565_to_rgb565_m2m() {
    let mut dma = Dma2d::new(MockRegs::new());
    let mut raw = vec![0u8; 32 * 16 * 2 + 4];
    let src_bytes = &*aligned4(&mut raw);
    let src = ImagePixels::new(ColorFormat::Rgb565, 32, 16, src_bytes);
    with_buf(ColorFormat::Rgb565, |buf, base| {
        let r = dma.blit(buf, Rect::from_xywh(20, 22, 32, 16), &src, Opa::COVER);
        assert_eq!(
            r,
            AccelResult::Done,
            "blits wait: the source borrow ends on return"
        );
        let regs = dma.regs();
        assert_eq!(mode(&dma), MODE_M2M);
        assert_eq!(regs.get(Reg::Fgmar), src_bytes.as_ptr() as usize as u32);
        assert_eq!(regs.get(Reg::Fgor), 0);
        assert_eq!(
            regs.get(Reg::Fgpfccr),
            CM_RGB565 | (AM_NO_MODIFY << 16) | (255 << 24)
        );
        assert_eq!(regs.get(Reg::Omar), (base + 2 * W as usize * 2 + 10 * 2) as u32);
        assert_eq!(regs.get(Reg::Oor), (W - 32) as u32);
        assert_eq!(regs.get(Reg::Nlr), nlr(32, 16));
    });
    assert!(!dma.is_busy());
}

#[test]
fn blit_rgb888_to_rgb565_uses_pfc() {
    let mut dma = Dma2d::new(MockRegs::new());
    let src_bytes = vec![0u8; 20 * 16 * 3];
    let src = ImagePixels::new(ColorFormat::Rgb888, 20, 16, &src_bytes);
    with_buf(ColorFormat::Rgb565, |buf, _| {
        assert_eq!(
            dma.blit(buf, Rect::from_xywh(10, 20, 20, 16), &src, Opa::COVER),
            AccelResult::Done
        );
    });
    assert_eq!(mode(&dma), MODE_M2M_PFC);
}

#[test]
fn blit_argb8888_on_rgb565_uses_blend_pfc() {
    let mut dma = Dma2d::new(MockRegs::new());
    let mut raw = vec![0u8; 32 * 16 * 4 + 4];
    let src_bytes = &*aligned4(&mut raw);
    let src = ImagePixels::new(ColorFormat::Argb8888, 32, 16, src_bytes);
    with_buf(ColorFormat::Rgb565, |buf, base| {
        let r = dma.blit(buf, Rect::from_xywh(10, 20, 32, 16), &src, Opa(200));
        assert_eq!(r, AccelResult::Done);
        let regs = dma.regs();
        assert_eq!(mode(&dma), MODE_M2M_BLEND);
        assert_eq!(
            regs.get(Reg::Fgpfccr),
            CM_ARGB8888 | (AM_MULTIPLY << 16) | (200 << 24)
        );
        assert_eq!(regs.get(Reg::Bgpfccr), CM_RGB565);
        assert_eq!(regs.get(Reg::Bgmar), base as u32);
        assert_eq!(regs.get(Reg::Opfccr), CM_RGB565);
    });
    // Opaque ARGB still blends (per-pixel alpha), with the alpha unmodified.
    with_buf(ColorFormat::Rgb565, |buf, _| {
        dma.blit(buf, Rect::from_xywh(10, 20, 32, 16), &src, Opa::COVER);
    });
    assert_eq!(mode(&dma), MODE_M2M_BLEND);
    assert_eq!(
        dma.regs().get(Reg::Fgpfccr),
        CM_ARGB8888 | (AM_NO_MODIFY << 16) | (255 << 24)
    );
}

#[test]
fn blit_l8_loads_gray_clut_first() {
    let mut dma = Dma2d::new(MockRegs::new());
    let src_bytes = vec![0u8; 32 * 16];
    let src = ImagePixels::new(ColorFormat::L8, 32, 16, &src_bytes);
    with_buf(ColorFormat::Rgb565, |buf, _| {
        assert_eq!(
            dma.blit(buf, Rect::from_xywh(10, 20, 32, 16), &src, Opa::COVER),
            AccelResult::Done
        );
    });
    let log = dma.regs().log();
    let clut_start = log
        .iter()
        .position(|a| matches!(a, Access::Write(Reg::Fgpfccr, v) if v & PFCCR_START != 0))
        .expect("CLUT load");
    let cr_start = log
        .iter()
        .position(|a| matches!(a, Access::Write(Reg::Cr, _)))
        .unwrap();
    assert!(clut_start < cr_start);
    assert!(matches!(log[clut_start], Access::Write(_, v) if v == CM_L8 | (255 << 8) | PFCCR_START));
    assert!(matches!(log[clut_start - 1], Access::Write(Reg::Fgcmar, _)));
    assert_eq!(mode(&dma), MODE_M2M_PFC);
}

#[test]
fn unsupported_sources_fall_back() {
    let mut dma = Dma2d::new(MockRegs::new());
    let data = vec![0u8; 32 * 16 * 4];
    let pal = vec![0u8; 16 * 4];
    let premul = ImagePixels::new(ColorFormat::Argb8888Premultiplied, 32, 16, &data);
    let a4 = ImagePixels::new(ColorFormat::A4, 32, 16, &data);
    let i4 = ImagePixels::from_parts(ColorFormat::I4, 32, 16, 16, &data, Some(&pal), None, false).unwrap();
    with_buf(ColorFormat::Rgb565, |buf, _| {
        for src in [premul, a4, i4] {
            let r = dma.blit(buf, Rect::from_xywh(10, 20, 32, 16), &src, Opa::COVER);
            assert_eq!(r, AccelResult::Unsupported, "{:?}", src.format);
        }
        // Size mismatch.
        let rgb = ImagePixels::new(ColorFormat::Rgb565, 32, 16, &data);
        assert_eq!(
            dma.blit(buf, Rect::from_xywh(10, 20, 31, 16), &rgb, Opa::COVER),
            AccelResult::Unsupported
        );
    });
    assert_eq!(dma.regs().starts(), 0);
}

#[test]
fn blend_a8_sets_fg_color_and_alpha_mode() {
    let mut dma = Dma2d::new(MockRegs::new());
    let alpha = vec![0x80u8; 40 * 20];
    with_buf(ColorFormat::Rgb565, |buf, base| {
        let area = Rect::from_xywh(10, 20, 30, 20);
        let r = dma.blend_a8(buf, area, Color::hex(0xAABBCC), &alpha, 40);
        assert_eq!(r, AccelResult::Done);
        let regs = dma.regs();
        assert_eq!(mode(&dma), MODE_M2M_BLEND);
        assert_eq!(regs.get(Reg::Fgpfccr), CM_A8 | (AM_NO_MODIFY << 16) | (255 << 24));
        assert_eq!(regs.get(Reg::Fgcolr), 0xAABBCC);
        assert_eq!(regs.get(Reg::Fgmar), alpha.as_ptr() as usize as u32);
        assert_eq!(regs.get(Reg::Fgor), 10);
        assert_eq!(regs.get(Reg::Bgmar), base as u32);
        assert_eq!(regs.get(Reg::Nlr), nlr(30, 20));
        // Short coverage buffer.
        assert_eq!(
            dma.blend_a8(buf, area, Color::WHITE, &alpha[..100], 40),
            AccelResult::Unsupported
        );
    });
}

#[test]
fn swapped_format_unsupported_falls_back() {
    let mut dma = Dma2d::new(MockRegs::new());
    for format in [ColorFormat::Rgb565Swapped, ColorFormat::L8, ColorFormat::I1] {
        let mut data = vec![0u8; (W * H * 2) as usize];
        let mut buf = DrawBuf::new_packed(&mut data, format, Rect::from_xywh(0, 0, W, H)).unwrap();
        let r = dma.fill(&mut buf, Rect::from_xywh(0, 0, W, H), Color::RED, Opa::COVER);
        assert_eq!(r, AccelResult::Unsupported, "{format:?}");
    }
    assert!(dma.regs().log().is_empty());
}

#[test]
fn small_area_below_min_px_unsupported() {
    let mut dma = Dma2d::new(MockRegs::new());
    assert_eq!(dma.min_px(), 256);
    with_buf(ColorFormat::Rgb565, |buf, _| {
        let r = dma.fill(buf, Rect::from_xywh(10, 20, 15, 17), Color::RED, Opa::COVER); // 255 px
        assert_eq!(r, AccelResult::Unsupported);
        let r = dma.fill(buf, Rect::from_xywh(10, 20, 16, 16), Color::RED, Opa::COVER); // 256 px
        assert_eq!(r, AccelResult::Queued);
    });
    let mut dma = Dma2d::new(MockRegs::new()).with_min_px(1024);
    with_buf(ColorFormat::Rgb565, |buf, _| {
        let r = dma.fill(buf, Rect::from_xywh(10, 20, 32, 16), Color::RED, Opa::COVER);
        assert_eq!(r, AccelResult::Unsupported);
    });
}

#[test]
fn area_outside_buffer_or_misaligned_unsupported() {
    let mut dma = Dma2d::new(MockRegs::new());
    with_buf(ColorFormat::Rgb565, |buf, _| {
        let r = dma.fill(buf, Rect::from_xywh(0, 0, W, H), Color::RED, Opa::COVER);
        assert_eq!(r, AccelResult::Unsupported, "not inside the buffer");
    });
    // A stride that is not a whole number of pixels.
    let mut raw = vec![0u8; (W as usize * 4 + 1) * H as usize + 4];
    let mut buf = DrawBuf::new(
        aligned4(&mut raw),
        ColorFormat::Argb8888,
        W as usize * 4 + 1,
        Rect::from_xywh(0, 0, W, H),
    )
    .unwrap();
    let r = dma.fill(&mut buf, Rect::from_xywh(0, 0, W, H), Color::RED, Opa::COVER);
    assert_eq!(r, AccelResult::Unsupported);
    assert_eq!(dma.regs().starts(), 0);
}

#[test]
fn new_op_waits_for_previous_and_clears_flags() {
    let mut dma = Dma2d::new(MockRegs::new().with_busy_reads(3));
    with_buf(ColorFormat::Rgb565, |buf, _| {
        dma.fill(buf, Rect::from_xywh(10, 20, 32, 16), Color::RED, Opa::COVER);
        assert!(dma.regs().is_running());
        let polls = dma.regs().cr_reads();
        dma.fill(buf, Rect::from_xywh(10, 36, 32, 16), Color::BLUE, Opa::COVER);
        assert_eq!(
            dma.regs().cr_reads() - polls,
            4,
            "spun on CR.START before reprogramming"
        );
    });
    let log = dma.regs().log();
    let starts: Vec<usize> = log
        .iter()
        .enumerate()
        .filter(|(_, a)| matches!(a, Access::Write(Reg::Cr, v) if v & CR_START != 0))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(starts.len(), 2);
    // Between the two starts, flags are cleared before the second programming begins.
    assert!(matches!(log[starts[0] + 1], Access::Write(Reg::Ifcr, 0x3F)));
    dma.wait();
    assert_eq!(dma.regs().get(Reg::Isr), 0);
}

#[test]
fn dcache_maintenance_around_transfers() {
    let mut dma = Dma2d::new(MockRegs::new());
    let src_bytes = vec![0u8; 16 * 16 * 3];
    let src = ImagePixels::new(ColorFormat::Rgb888, 16, 16, &src_bytes);
    with_buf(ColorFormat::Rgb565, |buf, base| {
        dma.blit(buf, Rect::from_xywh(10, 20, 16, 16), &src, Opa::COVER);
        let log = dma.regs().log();
        let dst_len = 15 * W as usize * 2 + 16 * 2;
        let clean = Access::Clean(src_bytes.as_ptr() as usize, src_bytes.len());
        let inval = Access::CleanInvalidate(base, dst_len);
        let start = log
            .iter()
            .position(|a| matches!(a, Access::Write(Reg::Cr, _)))
            .unwrap();
        let i_clean = log.iter().position(|a| *a == clean).expect("source cleaned");
        let i_inval: Vec<usize> = log
            .iter()
            .enumerate()
            .filter(|(_, a)| **a == inval)
            .map(|(i, _)| i)
            .collect();
        assert!(i_clean < start);
        assert_eq!(
            i_inval.len(),
            2,
            "destination cleaned+invalidated before and after"
        );
        assert!(i_inval[0] < start && i_inval[1] > start);
    });
}

#[test]
fn transfer_errors_are_counted() {
    let mut dma = Dma2d::new(MockRegs::new());
    let mut raw = vec![0u8; 16 * 16 * 2 + 4];
    let src = ImagePixels::new(ColorFormat::Rgb565, 16, 16, aligned4(&mut raw));
    with_buf(ColorFormat::Rgb565, |buf, _| {
        dma.regs_mut().fail_next_transfer(ISR_CEIF);
        let r = dma.blit(buf, Rect::from_xywh(10, 20, 16, 16), &src, Opa::COVER);
        assert_eq!(
            r,
            AccelResult::Unsupported,
            "configuration error: nothing written, software draws"
        );
        dma.regs_mut().fail_next_transfer(ISR_TEIF);
        dma.fill(buf, Rect::from_xywh(10, 20, 16, 16), Color::RED, Opa::COVER);
        dma.wait();
    });
    assert_eq!(dma.error_count(), 2);
}

#[test]
fn painter_integration_drop_waits() {
    use twine_render::{Painter, RenderCaches};
    let mut dma = Dma2d::new(MockRegs::new().with_busy_reads(5));
    let mut caches = RenderCaches::default();
    let mut raw = vec![0u8; (W * H * 2) as usize + 4];
    {
        let buf = DrawBuf::new_packed(
            aligned4(&mut raw),
            ColorFormat::Rgb565,
            Rect::from_xywh(0, 0, W, H),
        )
        .unwrap();
        let mut p = Painter::new(buf, &mut caches).with_accel(&mut dma);
        p.fill(Rect::from_xywh(0, 0, W, H), Color::RED, Opa::COVER);
    }
    assert!(!dma.is_busy());
    assert!(!dma.regs().is_running());
    assert_eq!(dma.release().starts(), 1);
}
