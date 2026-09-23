//! Property tests for `Rect` (coordinates in −10 000..10 000).

use proptest::prelude::*;
use twine_core::{Point, Rect};

fn coord() -> impl Strategy<Value = i32> {
    -10_000i32..10_000
}

fn rect() -> impl Strategy<Value = Rect> {
    (coord(), coord(), coord(), coord()).prop_map(|(a, b, c, d)| Rect::new(a, b, c, d))
}

/// Non-empty rectangles with moderate size (so that exhaustive pixel checks stay cheap).
fn small_rect() -> impl Strategy<Value = Rect> {
    (coord(), coord(), 1i32..200, 1i32..200).prop_map(|(x, y, w, h)| Rect::from_xywh(x, y, w, h))
}

proptest! {
    #[test]
    fn intersection_is_contained_in_both(a in rect(), b in rect()) {
        if let Some(i) = a.intersection(&b) {
            prop_assert!(a.contains_rect(&i));
            prop_assert!(b.contains_rect(&i));
            prop_assert!(!i.is_empty());
            prop_assert!(a.intersects(&b));
        } else {
            prop_assert!(!a.intersects(&b));
        }
    }

    #[test]
    fn union_contains_both(a in rect(), b in rect()) {
        let u = a.union(&b);
        prop_assert!(u.contains_rect(&a));
        prop_assert!(u.contains_rect(&b));
    }

    #[test]
    fn area_of_intersection_le_min_area(a in rect(), b in rect()) {
        let ia = a.intersection(&b).map_or(0, |i| i.area());
        prop_assert!(ia <= a.area().min(b.area()));
    }

    #[test]
    fn contains_point_iff_inside_bounds(r in rect(), x in coord(), y in coord()) {
        let inside = x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1;
        prop_assert_eq!(r.contains(Point::new(x, y)), inside);
    }

    #[test]
    fn rows_partition_area(r in small_rect(), n in 1i32..50) {
        let chunks: Vec<Rect> = r.rows(n).collect();
        let sum: u64 = chunks.iter().map(Rect::area).sum();
        prop_assert_eq!(sum, r.area());
        for (i, c) in chunks.iter().enumerate() {
            prop_assert!(c.height() <= n);
            prop_assert!(r.contains_rect(c));
            for d in &chunks[i + 1..] {
                prop_assert!(!c.intersects(d));
            }
        }
    }

    #[test]
    fn round_out_contains_original(r in rect(), align in 0u8..=64) {
        let o = r.round_out(align);
        prop_assert!(o.contains_rect(&r));
        if align > 1 && !r.is_empty() {
            let a = i32::from(align);
            prop_assert_eq!(o.x0.rem_euclid(a), 0);
            prop_assert_eq!(o.y1.rem_euclid(a), 0);
        }
    }

    #[test]
    fn subtract_partitions_difference(a in small_rect(), b in small_rect()) {
        let parts: Vec<Rect> = a.subtract(&b).collect();
        let inter = a.intersection(&b).map_or(0, |i| i.area());
        prop_assert_eq!(parts.iter().map(Rect::area).sum::<u64>(), a.area() - inter);
        for (i, p) in parts.iter().enumerate() {
            prop_assert!(a.contains_rect(p));
            prop_assert!(!p.intersects(&b));
            for q in &parts[i + 1..] {
                prop_assert!(!p.intersects(q));
            }
        }
    }
}
