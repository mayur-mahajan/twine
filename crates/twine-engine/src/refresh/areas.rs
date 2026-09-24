//! Dirty-area helpers: rounding to the display alignment and splitting into buffer-sized
//! horizontal chunks.

use twine_core::Rect;

/// Expands `r` to multiples of `align` (x0/y0 down, x1/y1 up; `align ≤ 1` leaves it
/// unchanged) and clips it to `bounds`.
#[must_use]
pub(crate) fn round_area(r: Rect, align: u8, bounds: Rect) -> Rect {
    r.round_out(align).intersection(&bounds).unwrap_or(Rect::ZERO)
}

#[cfg(test)]
/// Horizontal strips of `area`, each `area.width()` wide and at most `max_rows` rows (rounded
/// down to a multiple of `align`, at least `align`) tall, top to bottom.
pub(crate) fn chunks(area: Rect, max_rows: i32, align: u8) -> impl Iterator<Item = Rect> {
    area.rows(chunk_rows(max_rows, align))
}

/// The rows of one chunk: `max_rows` rounded down to a multiple of `align` (at least `align`,
/// at least 1).
pub(crate) fn chunk_rows(max_rows: i32, align: u8) -> i32 {
    let a = i32::from(align.max(1));
    ((max_rows / a) * a).max(a).max(1)
}

/// The chunk of `area` starting at row `y`.
pub(crate) fn chunk_at(area: Rect, y: i32, max_rows: i32, align: u8) -> Rect {
    let rows = chunk_rows(max_rows, align);
    Rect::new(area.x0, y, area.x1, (y + rows).min(area.y1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn round_expands_and_clips() {
        let b = Rect::new(0, 0, 100, 50);
        assert_eq!(round_area(Rect::new(3, 5, 10, 12), 8, b), Rect::new(0, 0, 16, 16));
        assert_eq!(round_area(Rect::new(3, 5, 10, 12), 1, b), Rect::new(3, 5, 10, 12));
        assert_eq!(
            round_area(Rect::new(90, 45, 99, 49), 8, b),
            Rect::new(88, 40, 100, 50)
        );
    }

    #[test]
    fn chunks_split_rows() {
        let a = Rect::new(0, 0, 320, 100);
        let c: Vec<Rect> = chunks(a, 40, 1).collect();
        assert_eq!(c.iter().map(Rect::height).collect::<Vec<_>>(), [40, 40, 20]);
        let c: Vec<Rect> = chunks(Rect::new(0, 8, 16, 40), 12, 8).collect();
        assert_eq!(c.iter().map(Rect::height).collect::<Vec<_>>(), [8, 8, 8, 8]);
        assert_eq!(chunk_rows(3, 8), 8);
        assert_eq!(chunk_at(a, 80, 40, 1), Rect::new(0, 80, 320, 100));
    }
}
