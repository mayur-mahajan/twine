//! The SVG path data grammar (`d` attribute): `M L H V C S Q T A Z` in absolute and relative
//! forms, implicit command repetition, smooth-curve reflection and compact number syntax
//! (`1.5.5`, `1-2`, arc flags without separators). On an error the path is kept up to the
//! last complete command (SVG error handling) and `false` is returned.

use twine_core::Fx;

use super::number::{Cursor, deg_to_angle};
use crate::geom::FxPoint;
use crate::path::Path;

/// Parses `d` into `path`. Returns `false` when the data had an error (the path holds the
/// commands before it).
pub(crate) fn parse_path_data(d: &str, path: &mut Path) -> bool {
    let mut c = Cursor::new(d);
    let mut cmd: u8 = 0;
    let mut cur = FxPoint::ZERO;
    let mut start = FxPoint::ZERO;
    // Last control point of the previous C/S (cubic) or Q/T (quadratic) command.
    let mut last_cubic: Option<FxPoint> = None;
    let mut last_quad: Option<FxPoint> = None;
    let mut first = true;
    loop {
        if c.at_end() {
            return true;
        }
        let Some(ch) = c.peek() else { return true };
        if ch.is_ascii_alphabetic() {
            c.pos += 1;
            cmd = ch;
        } else if cmd == 0 || matches!(cmd, b'Z' | b'z') {
            // Numbers without a command (or after Z) are an error.
            return false;
        }
        if first && !matches!(cmd, b'M' | b'm') {
            return false;
        }
        first = false;
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { cur } else { FxPoint::ZERO };
        let pt = |c: &mut Cursor<'_>| -> Option<FxPoint> {
            let x = c.number_sep()?;
            let y = c.number_sep()?;
            Some(FxPoint::new(base.x + x, base.y + y))
        };
        let ok = (|| -> Option<()> {
            match cmd.to_ascii_uppercase() {
                b'M' => {
                    let p = pt(&mut c)?;
                    path.move_to(p);
                    cur = p;
                    start = p;
                    // Further pairs are implicit line-tos.
                    cmd = if rel { b'l' } else { b'L' };
                    last_cubic = None;
                    last_quad = None;
                }
                b'L' => {
                    let p = pt(&mut c)?;
                    path.line_to(p);
                    cur = p;
                    last_cubic = None;
                    last_quad = None;
                }
                b'H' => {
                    let x = c.number_sep()?;
                    let p = FxPoint::new(if rel { cur.x + x } else { x }, cur.y);
                    path.line_to(p);
                    cur = p;
                    last_cubic = None;
                    last_quad = None;
                }
                b'V' => {
                    let y = c.number_sep()?;
                    let p = FxPoint::new(cur.x, if rel { cur.y + y } else { y });
                    path.line_to(p);
                    cur = p;
                    last_cubic = None;
                    last_quad = None;
                }
                b'C' => {
                    let (c1, c2, p) = (pt(&mut c)?, pt(&mut c)?, pt(&mut c)?);
                    path.cubic_to(c1, c2, p);
                    cur = p;
                    last_cubic = Some(c2);
                    last_quad = None;
                }
                b'S' => {
                    let (c2, p) = (pt(&mut c)?, pt(&mut c)?);
                    let c1 = reflect(last_cubic, cur);
                    path.cubic_to(c1, c2, p);
                    cur = p;
                    last_cubic = Some(c2);
                    last_quad = None;
                }
                b'Q' => {
                    let (q, p) = (pt(&mut c)?, pt(&mut c)?);
                    path.quad_to(q, p);
                    cur = p;
                    last_quad = Some(q);
                    last_cubic = None;
                }
                b'T' => {
                    let p = pt(&mut c)?;
                    let q = reflect(last_quad, cur);
                    path.quad_to(q, p);
                    cur = p;
                    last_quad = Some(q);
                    last_cubic = None;
                }
                b'A' => {
                    let rx = c.number_sep()?;
                    let ry = c.number_sep()?;
                    let rot = c.number_sep()?;
                    let large = c.flag()?;
                    let sweep = c.flag()?;
                    let p = pt(&mut c)?;
                    path.arc_to(
                        FxPoint::new(rx.abs(), ry.abs()),
                        deg_to_angle(rot),
                        large,
                        sweep,
                        p,
                    );
                    cur = p;
                    last_cubic = None;
                    last_quad = None;
                }
                b'Z' => {
                    path.close();
                    cur = start;
                    last_cubic = None;
                    last_quad = None;
                    c.skip_sep();
                }
                _ => return None,
            }
            Some(())
        })();
        if ok.is_none() {
            return false;
        }
    }
}

/// The reflection of `ctrl` about `cur` (or `cur` when there is no previous control point).
fn reflect(ctrl: Option<FxPoint>, cur: FxPoint) -> FxPoint {
    match ctrl {
        Some(q) => FxPoint::new(cur.x + cur.x - q.x, cur.y + cur.y - q.y),
        None => cur,
    }
}

/// Parses a `points` list (`polyline` / `polygon`) into `path`; an odd trailing number is
/// ignored.
pub(crate) fn parse_points(s: &str, path: &mut Path, close: bool) {
    let mut c = Cursor::new(s);
    let mut first = true;
    while let Some(x) = c.number_sep() {
        let Some(y) = c.number_sep() else { break };
        let p = FxPoint::new(x, y);
        if first {
            path.move_to(p);
            first = false;
        } else {
            path.line_to(p);
        }
    }
    if close && !first {
        path.close();
    }
}

/// Parses a list of numbers into `out` (up to its capacity); `None` on a syntax error.
pub(crate) fn parse_list<const N: usize>(s: &str, out: &mut heapless::Vec<Fx, N>) -> Option<()> {
    let mut c = Cursor::new(s);
    while !c.at_end() {
        let v = c.number()?;
        let _ = c.eat(b'%');
        c.skip_sep();
        if out.push(v).is_err() {
            break;
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::Verb;

    #[test]
    fn errors_keep_prefix() {
        let mut p = Path::new();
        assert!(!parse_path_data("M 1 2 L 3 4 L 5", &mut p));
        assert_eq!(p.verbs(), &[Verb::MoveTo, Verb::LineTo]);
        let mut p = Path::new();
        assert!(!parse_path_data("L 1 2", &mut p));
        assert!(p.is_empty());
        let mut p = Path::new();
        assert!(!parse_path_data("M1 2 X 3", &mut p));
        let mut p = Path::new();
        assert!(parse_path_data("  ", &mut p));
    }

    #[test]
    fn points_lists() {
        let mut p = Path::new();
        parse_points("0,0 10,0 10 10 5", &mut p, true);
        assert_eq!(
            p.verbs(),
            &[Verb::MoveTo, Verb::LineTo, Verb::LineTo, Verb::Close]
        );
    }
}
