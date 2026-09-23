//! In-house GIF decoder: [`GifDecoder`] (first frame, for static use) and [`GifPlayer`]
//! (animation with frame compositing).
//!
//! Supports GIF87a/GIF89a, global and local palettes, transparency, interlaced frames,
//! frames smaller than the canvas, the disposal methods *none*, *background* (cleared to
//! transparent, like browsers) and *previous*, per-frame delays and the NETSCAPE2.0 loop count.
//! Malformed data never panics: decoding stops at the first error.

use alloc::vec;
use alloc::vec::Vec;

use twine_core::{ColorFormat, Duration};
use twine_render::ImagePixels;

use crate::decoder::{Decoder, log_decoded, out_header, prepare_out};
use crate::{Error, ImageHeader};

/// Largest accepted canvas width or height.
pub const GIF_MAX_DIM: u16 = 16384;
/// Delay used for frames that specify none (like browsers).
pub const GIF_DEFAULT_DELAY: Duration = Duration::from_millis(100);

const MAX_CODES: usize = 4096;

/// How a frame is removed before the next one is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Disposal {
    /// Leave it in place.
    #[default]
    None,
    /// Clear its rectangle to transparent.
    Background,
    /// Restore the canvas as it was before the frame.
    Previous,
}

/// A frame rectangle on the canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FrameRect {
    x: u16,
    y: u16,
    w: u16,
    h: u16,
}

/// Canvas-level information from the logical screen descriptor.
#[derive(Clone, Copy, Debug)]
struct Screen {
    w: u16,
    h: u16,
    /// Byte range of the global palette.
    palette: Option<(usize, usize)>,
    /// Offset of the first block after the header.
    first_block: usize,
}

fn u16_at(b: &[u8], i: usize) -> Result<u16, Error> {
    match b.get(i..i + 2) {
        Some(s) => Ok(u16::from_le_bytes([s[0], s[1]])),
        None => Err(Error::Decode("gif truncated")),
    }
}

fn byte_at(b: &[u8], i: usize) -> Result<u8, Error> {
    b.get(i).copied().ok_or(Error::Decode("gif truncated"))
}

fn parse_screen(b: &[u8]) -> Result<Screen, Error> {
    if !(b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")) {
        return Err(Error::InvalidHeader);
    }
    let w = u16_at(b, 6)?;
    let h = u16_at(b, 8)?;
    if w == 0 || h == 0 || w > GIF_MAX_DIM || h > GIF_MAX_DIM {
        return Err(Error::InvalidHeader);
    }
    let flags = byte_at(b, 10)?;
    let mut pos = 13;
    let palette = if flags & 0x80 != 0 {
        let len = 3 << ((flags & 7) + 1);
        if b.len() < pos + len {
            return Err(Error::Decode("gif truncated"));
        }
        let r = (pos, pos + len);
        pos += len;
        Some(r)
    } else {
        None
    };
    Ok(Screen {
        w,
        h,
        palette,
        first_block: pos,
    })
}

/// Skips a chain of data sub-blocks starting at `pos`; returns the offset after the terminator.
fn skip_sub_blocks(b: &[u8], mut pos: usize) -> Result<usize, Error> {
    loop {
        let n = usize::from(byte_at(b, pos)?);
        pos += 1;
        if n == 0 {
            return Ok(pos);
        }
        pos += n;
        if pos > b.len() {
            return Err(Error::Decode("gif truncated"));
        }
    }
}

/// One parsed item of the block stream.
enum Block {
    /// A frame: its graphic control, rectangle, palette range, interlace flag, LZW minimum
    /// code size and the offset of its first data sub-block. `next` is the offset after it.
    Frame {
        ctrl: Control,
        rect: FrameRect,
        palette: Option<(usize, usize)>,
        interlaced: bool,
        min_code: u8,
        data: usize,
        next: usize,
    },
    /// The trailer (or the end of the data).
    End,
}

/// Graphic control extension values.
#[derive(Clone, Copy, Debug, Default)]
struct Control {
    disposal: Disposal,
    transparent: Option<u8>,
    delay_cs: u16,
}

/// Parses blocks from `pos` up to the next frame (extensions are applied to `ctrl` / `loops`).
fn next_frame(b: &[u8], mut pos: usize, loops: &mut Option<u16>) -> Result<Block, Error> {
    let mut ctrl = Control::default();
    loop {
        let Some(&kind) = b.get(pos) else {
            return Ok(Block::End);
        };
        match kind {
            0x3B => return Ok(Block::End),
            0x21 => {
                let label = byte_at(b, pos + 1)?;
                let body = pos + 2;
                if label == 0xF9 && byte_at(b, body)? >= 4 {
                    let flags = byte_at(b, body + 1)?;
                    ctrl.disposal = match (flags >> 2) & 7 {
                        2 => Disposal::Background,
                        3 => Disposal::Previous,
                        _ => Disposal::None,
                    };
                    ctrl.delay_cs = u16_at(b, body + 2)?;
                    ctrl.transparent = (flags & 1 != 0).then_some(byte_at(b, body + 4)?);
                } else if label == 0xFF
                    && byte_at(b, body)? == 11
                    && b.get(body + 1..body + 12) == Some(b"NETSCAPE2.0")
                    && byte_at(b, body + 12)? >= 3
                    && byte_at(b, body + 13)? == 1
                {
                    *loops = Some(u16_at(b, body + 14)?);
                }
                pos = skip_sub_blocks(b, body)?;
            }
            0x2C => {
                let rect = FrameRect {
                    x: u16_at(b, pos + 1)?,
                    y: u16_at(b, pos + 3)?,
                    w: u16_at(b, pos + 5)?,
                    h: u16_at(b, pos + 7)?,
                };
                let flags = byte_at(b, pos + 9)?;
                pos += 10;
                let palette = if flags & 0x80 != 0 {
                    let len = 3 << ((flags & 7) + 1);
                    if b.len() < pos + len {
                        return Err(Error::Decode("gif truncated"));
                    }
                    let r = (pos, pos + len);
                    pos += len;
                    Some(r)
                } else {
                    None
                };
                let min_code = byte_at(b, pos)?;
                if !(1..=11).contains(&min_code) {
                    return Err(Error::Decode("gif bad lzw code size"));
                }
                let data = pos + 1;
                let next = skip_sub_blocks(b, data)?;
                return Ok(Block::Frame {
                    ctrl,
                    rect,
                    palette,
                    interlaced: flags & 0x40 != 0,
                    min_code,
                    data,
                    next,
                });
            }
            _ => return Err(Error::Decode("gif unknown block")),
        }
    }
}

/// LZW dictionary and output stack (allocated once).
#[derive(Clone, Debug)]
struct Lzw {
    prefix: Vec<u16>,
    suffix: Vec<u8>,
    stack: Vec<u8>,
}

impl Lzw {
    fn new() -> Self {
        Self {
            prefix: vec![0; MAX_CODES],
            suffix: vec![0; MAX_CODES],
            stack: vec![0; MAX_CODES + 1],
        }
    }
}

/// Reads LZW codes across data sub-blocks.
struct Bits<'a> {
    b: &'a [u8],
    pos: usize,
    left_in_block: usize,
    acc: u32,
    n: u32,
    done: bool,
}

impl Bits<'_> {
    fn code(&mut self, size: u32) -> Option<u16> {
        while self.n < size {
            if self.done {
                return None;
            }
            if self.left_in_block == 0 {
                let len = usize::from(*self.b.get(self.pos)?);
                self.pos += 1;
                if len == 0 {
                    self.done = true;
                    return None;
                }
                self.left_in_block = len;
            }
            let v = *self.b.get(self.pos)?;
            self.pos += 1;
            self.left_in_block -= 1;
            self.acc |= u32::from(v) << self.n;
            self.n += 8;
        }
        let c = (self.acc & ((1 << size) - 1)) as u16;
        self.acc >>= size;
        self.n -= size;
        Some(c)
    }
}

/// Where decoded indices go: a frame rectangle on a canvas.
struct Target<'a> {
    canvas: &'a mut [u8],
    canvas_w: usize,
    canvas_h: usize,
    rect: FrameRect,
    interlaced: bool,
    palette: &'a [u8],
    transparent: Option<u8>,
    /// Pixel counter within the frame.
    i: usize,
}

impl Target<'_> {
    /// Frame row of the `row`-th decoded row (interlace passes: every 8th from 0, every 8th
    /// from 4, every 4th from 2, every 2nd from 1).
    fn frame_row(&self, row: usize) -> usize {
        let h = usize::from(self.rect.h);
        if !self.interlaced {
            return row;
        }
        let p1 = h.div_ceil(8);
        let p2 = (h + 3) / 8;
        let p3 = (h + 1) / 4;
        if row < p1 {
            row * 8
        } else if row < p1 + p2 {
            (row - p1) * 8 + 4
        } else if row < p1 + p2 + p3 {
            (row - p1 - p2) * 4 + 2
        } else {
            (row - p1 - p2 - p3) * 2 + 1
        }
    }

    #[inline]
    fn put(&mut self, idx: u8) {
        let fw = usize::from(self.rect.w);
        if fw == 0 {
            return;
        }
        let (fx, row) = (self.i % fw, self.i / fw);
        self.i += 1;
        if Some(idx) == self.transparent || row >= usize::from(self.rect.h) {
            return;
        }
        let x = usize::from(self.rect.x) + fx;
        let y = usize::from(self.rect.y) + self.frame_row(row);
        if x >= self.canvas_w || y >= self.canvas_h {
            return;
        }
        let o = (y * self.canvas_w + x) * 4;
        let c = usize::from(idx) * 3;
        let px = match self.palette.get(c..c + 3) {
            Some(p) => [p[2], p[1], p[0], 255],
            None => [0, 0, 0, 255],
        };
        self.canvas[o..o + 4].copy_from_slice(&px);
    }
}

/// Decodes one frame's LZW data (starting at `data`) into `t`.
fn decode_lzw(b: &[u8], data: usize, min_code: u8, lzw: &mut Lzw, t: &mut Target<'_>) {
    let total = usize::from(t.rect.w) * usize::from(t.rect.h);
    let clear = 1u16 << min_code;
    let eoi = clear + 1;
    let mut size = u32::from(min_code) + 1;
    let mut next = clear + 2;
    let mut old: Option<u16> = None;
    let mut first = 0u8;
    let mut bits = Bits {
        b,
        pos: data,
        left_in_block: 0,
        acc: 0,
        n: 0,
        done: false,
    };
    for i in 0..clear {
        lzw.suffix[usize::from(i)] = i as u8;
    }
    while t.i < total {
        let Some(code) = bits.code(size) else { return };
        if code == clear {
            size = u32::from(min_code) + 1;
            next = clear + 2;
            old = None;
            continue;
        }
        if code == eoi {
            return;
        }
        let Some(prev) = old else {
            if code >= clear {
                return;
            }
            first = code as u8;
            t.put(first);
            old = Some(code);
            continue;
        };
        // Expand `code` (or `prev` + its first byte for the KwKwK case) onto the stack.
        let mut sp = 0;
        let mut c = match code.cmp(&next) {
            core::cmp::Ordering::Less => code,
            core::cmp::Ordering::Equal => {
                lzw.stack[0] = first;
                sp = 1;
                prev
            }
            core::cmp::Ordering::Greater => return,
        };
        while c >= clear {
            if sp >= MAX_CODES {
                return;
            }
            lzw.stack[sp] = lzw.suffix[usize::from(c)];
            sp += 1;
            c = lzw.prefix[usize::from(c)];
        }
        first = c as u8;
        lzw.stack[sp] = first;
        sp += 1;
        while sp > 0 {
            sp -= 1;
            t.put(lzw.stack[sp]);
        }
        if usize::from(next) < MAX_CODES {
            lzw.prefix[usize::from(next)] = prev;
            lzw.suffix[usize::from(next)] = first;
            next += 1;
            if u32::from(next) == 1 << size && size < 12 {
                size += 1;
            }
        }
        old = Some(code);
    }
}

/// Decodes the first frame of GIF files (static use; see [`GifPlayer`] for animation).
#[derive(Clone, Copy, Debug, Default)]
pub struct GifDecoder;

impl Decoder for GifDecoder {
    fn name(&self) -> &'static str {
        "gif"
    }

    fn probe(&self, bytes: &[u8]) -> Option<ImageHeader> {
        let s = parse_screen(bytes).ok()?;
        Some(out_header(s.w, s.h, true))
    }

    fn decode(&self, bytes: &[u8], out: &mut Vec<u8>) -> Result<ImageHeader, Error> {
        let s = parse_screen(bytes)?;
        let header = out_header(s.w, s.h, true);
        let n = usize::from(s.w) * usize::from(s.h) * 4;
        prepare_out(out, n)?;
        let mut lzw = Lzw::new();
        let mut loops = None;
        match next_frame(bytes, s.first_block, &mut loops)? {
            Block::Frame {
                ctrl,
                rect,
                palette,
                interlaced,
                min_code,
                data,
                ..
            } => draw_frame(bytes, &s, out, &mut lzw, ctrl, rect, palette, interlaced, min_code, data),
            Block::End => return Err(Error::Decode("gif has no frames")),
        }
        log_decoded("gif", header, n);
        Ok(header)
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_frame(
    b: &[u8],
    s: &Screen,
    canvas: &mut [u8],
    lzw: &mut Lzw,
    ctrl: Control,
    rect: FrameRect,
    palette: Option<(usize, usize)>,
    interlaced: bool,
    min_code: u8,
    data: usize,
) {
    let pal = palette.or(s.palette).map_or(&[][..], |(a, e)| &b[a..e]);
    let mut t = Target {
        canvas,
        canvas_w: usize::from(s.w),
        canvas_h: usize::from(s.h),
        rect,
        interlaced,
        palette: pal,
        transparent: ctrl.transparent,
        i: 0,
    };
    decode_lzw(b, data, min_code, lzw, &mut t);
}

/// Plays an animated GIF: [`advance`](Self::advance) composites the next frame onto an
/// `Argb8888` canvas and returns how long to show it.
///
/// All memory (canvas, LZW tables, the *previous*-disposal backup when a frame needs it) is
/// allocated in [`new`](Self::new); advancing never allocates.
///
/// ```
/// use twine_image::decoders::gif::GifPlayer;
/// # // A 1×1 single-frame GIF (white pixel).
/// # static GIF: [u8; 35] = [
/// #     b'G', b'I', b'F', b'8', b'9', b'a', 1, 0, 1, 0, 0x80, 0, 0,
/// #     255, 255, 255, 0, 0, 0,
/// #     0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x44, 0x01, 0, 0x3B,
/// # ];
/// let mut gif = GifPlayer::new(&GIF).unwrap();
/// assert_eq!(gif.frame_count(), 1);
/// let delay = gif.advance();
/// assert_eq!(delay.as_millis(), 100); // no delay given → 100 ms
/// assert_eq!(gif.pixels().row(0), &[255, 255, 255, 255]);
/// ```
#[derive(Clone, Debug)]
pub struct GifPlayer {
    bytes: &'static [u8],
    screen: Screen,
    canvas: Vec<u8>,
    /// Backup for `Disposal::Previous` (empty when no frame needs it).
    prev: Vec<u8>,
    lzw: Lzw,
    /// Offset of the next block to parse.
    pos: usize,
    /// Disposal of the frame on screen, applied before the next one.
    pending: Option<(Disposal, FrameRect)>,
    frames: usize,
    loops: Option<u16>,
    loops_done: u16,
    finished: bool,
    frame_index: usize,
    next_index: usize,
}

impl GifPlayer {
    /// Parses the GIF and allocates everything needed to play it. The canvas starts
    /// transparent; call [`advance`](Self::advance) to show the first frame.
    pub fn new(bytes: &'static [u8]) -> Result<Self, Error> {
        let screen = parse_screen(bytes)?;
        // Scan once: frame count, loop count, whether `previous` disposal is used.
        let (mut pos, mut frames, mut loops, mut needs_prev) = (screen.first_block, 0, None, false);
        while let Ok(Block::Frame { ctrl, next, .. }) = next_frame(bytes, pos, &mut loops) {
            frames += 1;
            needs_prev |= ctrl.disposal == Disposal::Previous;
            pos = next;
        }
        if frames == 0 {
            return Err(Error::Decode("gif has no frames"));
        }
        let n = usize::from(screen.w) * usize::from(screen.h) * 4;
        let mut canvas = Vec::new();
        prepare_out(&mut canvas, n)?;
        let mut prev = Vec::new();
        if needs_prev {
            prepare_out(&mut prev, n)?;
        }
        twine_core::debug!(target: "twine::image", "gif {}x{}: {} frames, loop {:?}", screen.w, screen.h, frames, loops);
        Ok(Self {
            bytes,
            screen,
            canvas,
            prev,
            lzw: Lzw::new(),
            pos: screen.first_block,
            pending: None,
            frames,
            loops,
            loops_done: 0,
            finished: false,
            frame_index: 0,
            next_index: 0,
        })
    }

    /// Canvas header (`Argb8888`, canvas size).
    #[must_use]
    pub fn header(&self) -> ImageHeader {
        out_header(self.screen.w, self.screen.h, true)
    }

    /// Number of frames.
    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.frames
    }

    /// Index of the frame on the canvas (0-based; 0 before the first `advance`).
    #[must_use]
    pub fn frame_index(&self) -> usize {
        self.frame_index
    }

    /// The NETSCAPE2.0 loop count: `None` without the extension (play once), `Some(0)` loops
    /// forever, `Some(n)` repeats `n` more times.
    #[must_use]
    pub fn loop_count(&self) -> Option<u16> {
        self.loops
    }

    /// Whether the last loop has ended (the last frame stays on the canvas).
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// The canvas.
    #[must_use]
    pub fn pixels(&self) -> ImagePixels<'_> {
        ImagePixels::new(ColorFormat::Argb8888, self.screen.w, self.screen.h, &self.canvas)
    }

    /// Restarts from the first frame with a transparent canvas.
    pub fn reset(&mut self) {
        self.canvas.fill(0);
        self.pos = self.screen.first_block;
        self.pending = None;
        self.loops_done = 0;
        self.finished = false;
        self.frame_index = 0;
        self.next_index = 0;
    }

    /// Composites the next frame onto the canvas and returns its delay (0 → 100 ms). After
    /// the last loop the canvas is left unchanged and [`GIF_DEFAULT_DELAY`] returned.
    pub fn advance(&mut self) -> Duration {
        if self.finished {
            return GIF_DEFAULT_DELAY;
        }
        self.dispose();
        let mut ignored = None;
        let mut restarted = false;
        loop {
            match next_frame(self.bytes, self.pos, &mut ignored) {
                Ok(Block::Frame {
                    ctrl,
                    rect,
                    palette,
                    interlaced,
                    min_code,
                    data,
                    next,
                }) => {
                    if ctrl.disposal == Disposal::Previous && !self.prev.is_empty() {
                        self.prev.copy_from_slice(&self.canvas);
                    }
                    draw_frame(
                        self.bytes,
                        &self.screen,
                        &mut self.canvas,
                        &mut self.lzw,
                        ctrl,
                        rect,
                        palette,
                        interlaced,
                        min_code,
                        data,
                    );
                    self.pending = Some((ctrl.disposal, rect));
                    self.pos = next;
                    self.frame_index = self.next_index;
                    self.next_index += 1;
                    return if ctrl.delay_cs == 0 {
                        GIF_DEFAULT_DELAY
                    } else {
                        Duration::from_millis(u64::from(ctrl.delay_cs) * 10)
                    };
                }
                Ok(Block::End) | Err(_) => {
                    if restarted || !self.next_loop() {
                        self.finished = !restarted;
                        return GIF_DEFAULT_DELAY;
                    }
                    self.canvas.fill(0);
                    self.pos = self.screen.first_block;
                    self.next_index = 0;
                    restarted = true;
                }
            }
        }
    }

    /// Whether another loop is due (counts it).
    fn next_loop(&mut self) -> bool {
        match self.loops {
            Some(0) => true,
            Some(n) if self.loops_done < n => {
                self.loops_done += 1;
                true
            }
            _ => false,
        }
    }

    /// Applies the disposal of the frame on the canvas.
    fn dispose(&mut self) {
        let Some((d, r)) = self.pending.take() else {
            return;
        };
        if d == Disposal::None {
            return;
        }
        let (cw, ch) = (usize::from(self.screen.w), usize::from(self.screen.h));
        let x0 = usize::from(r.x).min(cw);
        let x1 = (usize::from(r.x) + usize::from(r.w)).min(cw);
        for y in usize::from(r.y)..(usize::from(r.y) + usize::from(r.h)).min(ch) {
            let span = (y * cw + x0) * 4..(y * cw + x1) * 4;
            match d {
                Disposal::Background => self.canvas[span].fill(0),
                Disposal::Previous if !self.prev.is_empty() => {
                    self.canvas[span.clone()].copy_from_slice(&self.prev[span]);
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interlace_row_mapping_is_a_permutation() {
        for h in 1..40u16 {
            let t = Target {
                canvas: &mut [],
                canvas_w: 0,
                canvas_h: 0,
                rect: FrameRect { x: 0, y: 0, w: 1, h },
                interlaced: true,
                palette: &[],
                transparent: None,
                i: 0,
            };
            let mut rows: Vec<usize> = (0..usize::from(h)).map(|r| t.frame_row(r)).collect();
            rows.sort_unstable();
            assert_eq!(rows, (0..usize::from(h)).collect::<Vec<_>>(), "h={h}");
        }
    }
}
