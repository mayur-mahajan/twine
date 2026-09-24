//! SVG parser: path data, colors, gradients, transforms, icons, robustness.
#![allow(clippy::unreadable_literal)] // colors read best as 0xRRGGBB
#![allow(clippy::manual_assert_eq)]

use proptest::prelude::*;
use twine_core::{Color, ColorFormat, Fx, Opa, Point, Rect};
use twine_render::{FillRule, GradExtend};
use twine_testing::{RenderHarness, assert_render_snapshot};
use twine_vector::{FxPoint, LineCap, LineJoin, Paint, PathEl, SvgError, parse_svg};

fn p(x: i32, y: i32) -> FxPoint {
    FxPoint::from_int(x, y)
}

fn doc(body: &str) -> twine_vector::SvgDocument {
    let s = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">{body}</svg>"#);
    parse_svg(s.as_bytes()).unwrap()
}

#[test]
fn parse_path_d_all_commands() {
    use PathEl::{Close, CubicTo, LineTo, MoveTo, QuadTo};
    let d = doc(
        r#"<path d="M10 20 L30 40 H50 V60 C1 2 3 4 5 6 S7 8 9 10 Q11 12 13 14 T15 16 A5 5 0 0 1 25 16 Z
                m1 1 l2 2 h1 v1 c1 1 2 2 3 3 s1 1 2 2 q1 1 2 2 t1 1 a1 1 0 1 0 2 2 z
                M0 0 1 1 2 2 m1-1.5.5 3e1z"/>"#,
    );
    let (path, _) = d.scene.items().next().unwrap();
    let els: Vec<PathEl> = path.iter().collect();
    let golden_prefix = [
        MoveTo(p(10, 20)),
        LineTo(p(30, 40)),
        LineTo(p(50, 40)),
        LineTo(p(50, 60)),
        CubicTo(p(1, 2), p(3, 4), p(5, 6)),
        // S reflects (3, 4) about (5, 6).
        CubicTo(p(7, 8), p(7, 8), p(9, 10)),
        QuadTo(p(11, 12), p(13, 14)),
        // T reflects (11, 12) about (13, 14).
        QuadTo(p(15, 16), p(15, 16)),
    ];
    assert_eq!(&els[..golden_prefix.len()], &golden_prefix);
    // The arc ends exactly at (25, 16), then Z.
    let close1 = els.iter().position(|e| *e == Close).unwrap();
    assert!(matches!(els[close1 - 1], CubicTo(_, _, e) if e == p(25, 16)));
    // Relative commands continue from the sub-path start (10, 20) after Z.
    let rel = &els[close1 + 1..];
    assert_eq!(rel[0], MoveTo(p(11, 21)));
    assert_eq!(rel[1], LineTo(p(13, 23)));
    assert_eq!(rel[2], LineTo(p(14, 23)));
    assert_eq!(rel[3], LineTo(p(14, 24)));
    assert_eq!(rel[4], CubicTo(p(15, 25), p(16, 26), p(17, 27)));
    // s: first control = reflection of (16, 26) about (17, 27) = (18, 28).
    assert_eq!(rel[5], CubicTo(p(18, 28), p(18, 28), p(19, 29)));
    assert_eq!(rel[6], QuadTo(p(20, 30), p(21, 31)));
    assert_eq!(rel[7], QuadTo(p(22, 32), p(22, 32)));
    let close2 = close1 + 1 + rel.iter().position(|e| *e == Close).unwrap();
    assert!(matches!(els[close2 - 1], CubicTo(_, _, e) if e == p(24, 34)));
    // Implicit line-tos after M, compact numbers "1-1.5.5" and exponents.
    let tail = &els[close2 + 1..];
    assert_eq!(tail[0], MoveTo(p(0, 0)));
    assert_eq!(tail[1], LineTo(p(1, 1)));
    assert_eq!(tail[2], LineTo(p(2, 2)));
    assert_eq!(
        tail[3],
        MoveTo(FxPoint::new(Fx::from_int(3), Fx::from_ratio(1, 2)))
    );
    assert_eq!(
        tail[4],
        LineTo(FxPoint::new(Fx::from_ratio(7, 2), Fx::from_ratio(61, 2)))
    );
    assert_eq!(tail.last(), Some(&Close));
}

fn fill_of(d: &twine_vector::SvgDocument, i: usize) -> Option<Paint> {
    d.scene
        .items()
        .nth(i)
        .and_then(|(_, dsc)| dsc.fill.clone().map(|f| f.0))
}

#[test]
fn parse_colors() {
    let d = doc(r##"<rect width="1" height="1" fill="#f80"/>
           <rect width="1" height="1" fill="#1E88E5"/>
           <rect width="1" height="1" fill="rgb(10, 20, 30)"/>
           <rect width="1" height="1" fill="rgb(100%,0%,50%)"/>
           <rect width="1" height="1" fill="rebeccapurple" style="fill: tomato"/>
           <g color="teal"><rect width="1" height="1" fill="currentColor"/></g>
           <rect width="1" height="1" fill="none" stroke="navy"/>
           <rect width="1" height="1" fill="bogus"/>"##);
    let solid = |c: u32| Some(Paint::Solid(Color::hex(c)));
    assert_eq!(fill_of(&d, 0), solid(0xFF8800));
    assert_eq!(fill_of(&d, 1), solid(0x1E88E5));
    assert_eq!(fill_of(&d, 2), solid(0x0A141E));
    assert_eq!(fill_of(&d, 3), solid(0xFF007F));
    assert_eq!(fill_of(&d, 4), solid(0xFF6347));
    assert_eq!(fill_of(&d, 5), solid(0x008080));
    assert_eq!(fill_of(&d, 6), None);
    assert_eq!(
        d.scene.items().nth(6).unwrap().1.stroke.as_ref().unwrap().0,
        Paint::Solid(Color::hex(0x000080))
    );
    // An invalid color keeps the inherited default (black).
    assert_eq!(fill_of(&d, 7), solid(0x000000));
}

#[test]
fn gradient_href_inheritance() {
    let d = doc(r##"<defs>
              <linearGradient id="base" x1="0" y1="0" x2="0" y2="1" spreadMethod="reflect">
                <stop offset="0" stop-color="red"/>
                <stop offset="50%" style="stop-color: lime; stop-opacity: 0.5"/>
                <stop offset="0.25" stop-color="blue"/>
              </linearGradient>
              <linearGradient id="child" xlink:href="#base" x2="1"/>
              <radialGradient id="rad" href="#child" cx="0.25" gradientUnits="userSpaceOnUse" gradientTransform="scale(2)"/>
            </defs>
            <rect x="10" y="10" width="20" height="40" fill="url(#child)"/>
            <rect x="10" y="10" width="20" height="40" fill="url(#rad)"/>
            <rect x="10" y="10" width="20" height="40" fill="url(#missing) green"/>
            <rect x="10" y="10" width="20" height="40" fill="url(#missing)"/>"##);
    let Some(Paint::Linear {
        start,
        end,
        stops,
        extend,
        transform,
    }) = fill_of(&d, 0)
    else {
        panic!("linear");
    };
    // x2 from the child, y2 from the parent; stops and spread inherited.
    assert_eq!((start, end), (p(0, 0), p(1, 1)));
    assert_eq!(extend, GradExtend::Reflect);
    let s = stops.as_slice();
    assert_eq!(s.len(), 3);
    assert_eq!((s[0].color, s[0].frac), (Color::RED, 0));
    assert_eq!(
        (s[1].color, s[1].opa, s[1].frac),
        (Color::hex(0x00FF00), Opa(128), 128)
    );
    // Offsets never decrease.
    assert_eq!(s[2].frac, 128);
    // objectBoundingBox: unit square → the rect.
    assert_eq!(transform.map_point(Point::new(1, 1)), Point::new(30, 50));
    let Some(Paint::Radial {
        center,
        radius,
        stops,
        transform,
        ..
    }) = fill_of(&d, 1)
    else {
        panic!("radial");
    };
    // Geometry is not inherited across kinds, stops are; user space.
    assert_eq!(center, FxPoint::new(Fx::from_ratio(1, 4), Fx::from_int(50)));
    assert_eq!(radius, Fx::from_int(50));
    assert_eq!(stops.as_slice().len(), 3);
    assert_eq!(transform.map_point(Point::new(1, 1)), Point::new(2, 2));
    assert_eq!(fill_of(&d, 2), Some(Paint::Solid(Color::hex(0x008000))));
    assert_eq!(d.scene.len(), 3, "a missing url without fallback paints nothing");
}

#[test]
fn transform_list_order() {
    let d = doc(r#"<g transform="translate(10,20)">
             <rect width="1" height="1" transform="scale(2) rotate(90)"/>
             <g transform="matrix(1 0 0 1 5 5)"><circle r="1" transform="skewX(45)"/></g>
           </g>"#);
    let (_, dsc) = d.scene.items().next().unwrap();
    // rotate first, then scale, then the group's translate: (1, 0) → (0, 1) → (0, 2) → (10, 22).
    assert_eq!(dsc.transform.map_point(Point::new(1, 0)), Point::new(10, 22));
    let (_, dsc) = d.scene.items().nth(1).unwrap();
    // skewX(45): (0, 1) → (1, 1); + (5, 5) + (10, 20).
    assert_eq!(dsc.transform.map_point(Point::new(0, 1)), Point::new(16, 26));
}

#[test]
fn styles_and_strokes() {
    let d = doc(
        r#"<g opacity="0.5" stroke-width="4" style="stroke-linejoin:round">
             <polyline points="0,0 10,10 20,0" fill="red" stroke="black" stroke-linecap="square"
                       stroke-dasharray="3 1 2" stroke-dashoffset="1" stroke-miterlimit="8"
                       fill-opacity="0.5" stroke-opacity="25%" opacity="0.5" fill-rule="evenodd"/>
             <line x1="0" y1="0" x2="10" y2="0" stroke="blue"/>
           </g>
           <path d="M0 0h10v10z" display="none"/>
           <g display="none"><path d="M0 0h10v10z"/></g>
           <text>ignored</text>"#,
    );
    assert_eq!(d.scene.len(), 2);
    let (_, dsc) = d.scene.items().next().unwrap();
    assert_eq!(dsc.opa, Opa(64));
    assert_eq!(dsc.fill_opa, Opa(128));
    assert_eq!(dsc.stroke_opa, Opa(64));
    assert_eq!(dsc.fill.as_ref().unwrap().1, FillRule::EvenOdd);
    let s = &dsc.stroke.as_ref().unwrap().1;
    assert_eq!(
        (s.width, s.join, s.cap, s.miter_limit),
        (Fx::from_int(4), LineJoin::Round, LineCap::Square, Fx::from_int(8))
    );
    let dash = s.dash.as_ref().unwrap();
    assert_eq!(dash.pattern.len(), 3);
    assert_eq!(dash.offset, Fx::ONE);
    // Lines are never filled.
    let (_, line) = d.scene.items().nth(1).unwrap();
    assert!(line.fill.is_none() && line.stroke.is_some());
}

#[test]
fn deep_nesting_errors_cleanly() {
    let mut s = String::from("<svg>");
    for _ in 0..40 {
        s.push_str("<g>");
    }
    for _ in 0..40 {
        s.push_str("</g>");
    }
    s.push_str("</svg>");
    assert_eq!(parse_svg(s.as_bytes()), Err(SvgError::TooDeep));
    // 30 levels are fine.
    let mut s = String::from("<svg>");
    for _ in 0..30 {
        s.push_str("<g>");
    }
    s.push_str(r#"<rect width="5" height="5"/>"#);
    for _ in 0..30 {
        s.push_str("</g>");
    }
    s.push_str("</svg>");
    assert_eq!(parse_svg(s.as_bytes()).unwrap().scene.len(), 1);
}

#[test]
fn structural_errors() {
    assert_eq!(parse_svg(b"<g/>"), Err(SvgError::NotSvg));
    assert_eq!(parse_svg(b""), Err(SvgError::NotSvg));
    assert_eq!(parse_svg(b"<svg><g></svg>"), Err(SvgError::Malformed(0)));
    assert_eq!(parse_svg(b"<svg><g>"), Err(SvgError::UnexpectedEof));
    assert_eq!(parse_svg(b"<svg/><svg/>"), Err(SvgError::Malformed(0)));
    assert_eq!(parse_svg(&[0xFF, 0xFE]), Err(SvgError::InvalidUtf8));
    assert!(parse_svg(b"<svg><!-- unterminated").is_err());
    // Unsupported elements and broken attributes are ignored, not errors.
    let d =
        parse_svg(br##"<svg><use href="#a"/><rect width="x" height="1"/><path d="M0 0 L"/></svg>"##).unwrap();
    assert_eq!(d.scene.len(), 1);
}

/// Icons from Google's Material Design Icons (Apache License 2.0,
/// <https://github.com/google/material-design-icons>), 24 × 24 view box, reproduced path data.
const MATERIAL_ICONS: [(&str, &str); 10] = [
    ("home", "M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z"),
    (
        "star",
        "M12 17.27L18.18 21l-1.64-7.03L22 9.24l-7.19-.61L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21z",
    ),
    (
        "favorite",
        "M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z",
    ),
    ("check", "M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z"),
    (
        "close",
        "M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z",
    ),
    ("add", "M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z"),
    ("menu", "M3 18h18v-2H3v2zm0-5h18v-2H3v2zm0-7v2h18V6H3z"),
    (
        "search",
        "M15.5 14h-.79l-.28-.27C15.41 12.59 16 11.11 16 9.5 16 5.91 13.09 3 9.5 3S3 5.91 3 9.5 5.91 16 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z",
    ),
    (
        "info",
        "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-6h2v6zm0-8h-2V7h2v2z",
    ),
    (
        "delete",
        "M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z",
    ),
];

#[test]
fn material_icon_snapshots() {
    for (name, d) in MATERIAL_ICONS {
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" height="24" viewBox="0 0 24 24" width="24"><path d="M0 0h24v24H0z" fill="none"/><path d="{d}" fill="#37474F"/></svg>"##
        );
        let doc = parse_svg(svg.as_bytes()).unwrap();
        assert_eq!(doc.scene.len(), 1, "{name}");
        let mut h = RenderHarness::new(96, 60, ColorFormat::Rgb565);
        h.paint(|pa| {
            doc.render(pa, Rect::new(2, 6, 50, 54));
            doc.render(pa, Rect::new(60, 18, 84, 42));
        });
        assert_render_snapshot!(h, &format!("svg_icon_{name}"));
    }
}

#[test]
fn svg_features_snapshot() {
    let src = br##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="160" height="100">
  <defs>
    <linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#1E88E5"/><stop offset="1" stop-color="#E3F2FD"/>
    </linearGradient>
    <radialGradient id="sun" cx="0.4" cy="0.4" r="0.6" fx="0.3" fy="0.3">
      <stop offset="0" stop-color="#FFF59D"/><stop offset="1" stop-color="#FB8C00"/>
    </radialGradient>
  </defs>
  <rect width="160" height="100" rx="12" fill="url(#sky)"/>
  <circle cx="120" cy="30" r="18" fill="url(#sun)" stroke="#E65100" stroke-width="2"/>
  <g transform="translate(0 60)" fill="#43A047" stroke="#1B5E20" stroke-width="1.5" stroke-linejoin="round">
    <polygon points="0,40 30,5 55,40"/>
    <polygon points="35,40 70,0 110,40" opacity="0.8"/>
  </g>
  <path d="M10 20 q 20 -15 40 0 t 40 0" fill="none" stroke="white" stroke-width="3" stroke-dasharray="6 4" stroke-linecap="round"/>
  <ellipse cx="80" cy="85" rx="30" ry="6" fill="black" fill-opacity="0.2"/>
  <line x1="130" y1="60" x2="150" y2="95" stroke="#6D4C41" stroke-width="4" stroke-linecap="round"/>
  <polyline points="140,70 150,62 158,70" fill="none" stroke="#2E7D32" stroke-width="3" stroke-linejoin="miter"/>
</svg>"##;
    let doc = parse_svg(src).unwrap();
    let mut h = RenderHarness::new(160, 100, ColorFormat::Rgb565);
    h.paint(|pa| doc.render(pa, Rect::new(0, 0, 160, 100)));
    assert_render_snapshot!(h, "svg_features");
    // Chunked rendering of a document is identical.
    let mut c = RenderHarness::new(160, 100, ColorFormat::Rgb565);
    c.paint_chunked(9, |pa| doc.render(pa, Rect::new(0, 0, 160, 100)));
    assert!(h.data() == c.data());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        let _ = parse_svg(&bytes);
    }

    #[test]
    fn random_svg_like_never_panics(
        parts in prop::collection::vec(prop::sample::select(vec![
            "<svg", ">", "</svg>", "<g", "</g>", "<path", " d=\"", "M", "m", "L", "C", "A", "z", "Q", "T", "S", "H", "v",
            "1", "-2.5", ".5e3", "e", ",", " ", "\"", "'", "/>", "<rect", " width=", " rx=", "<circle", " r=", "<!--", "-->",
            " fill=\"url(#a)\"", "<linearGradient id=\"a\"", " href=\"#a\"", "<stop", " offset=", " transform=\"", "rotate(",
            "matrix(", ")", " style=\"", "fill:", ";", "&amp;", "&#x41;", "&", "<![CDATA[", "]]>", "<?", "?>", "<!DOCTYPE [",
            "]", "%", " stroke-dasharray=\"", " stroke=\"red\"", " points=\"", "<polygon", "<polyline", " viewBox=\"0 0 1e9 -3\"",
        ]), 0..60)
    ) {
        let s: String = parts.concat();
        let _ = parse_svg(s.as_bytes());
    }

    /// Well-formed documents with random elements, attributes and values: they parse, and
    /// whatever parses renders (in chunks identical to one pass).
    #[test]
    fn random_documents_parse_and_render(
        elems in prop::collection::vec(
            (
                prop::sample::select(vec!["path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "g", "linearGradient", "radialGradient", "stop", "text", "defs"]),
                prop::collection::vec(
                    (
                        prop::sample::select(vec!["d", "x", "y", "width", "height", "rx", "ry", "cx", "cy", "r", "fx", "fy", "x1", "y1", "x2", "y2",
                            "points", "fill", "stroke", "stroke-width", "stroke-dasharray", "stroke-linejoin", "stroke-linecap", "opacity",
                            "fill-opacity", "fill-rule", "transform", "style", "id", "href", "offset", "stop-color", "gradientUnits",
                            "gradientTransform", "spreadMethod", "stroke-miterlimit", "display"]),
                        prop::collection::vec(prop::sample::select(vec![
                            "M", "m", "L", "l", "H", "h", "V", "v", "C", "c", "S", "s", "Q", "q", "T", "t", "A", "a", "Z", " ", ",",
                            "0", "1", "-3", "12.5", ".5", "1e2", "-1e-2", "40", "%", "px", "#a", "url(#a)", "url(#b) red", "red", "#123",
                            "rgb(1,2,3)", "none", "round", "bevel", "evenodd", "translate(", "scale(", "rotate(", "skewX(", "matrix(", ")",
                            "fill:blue;", "stroke:green", "userSpaceOnUse", "reflect", "repeat", "a", "b", "inherit", "currentColor",
                        ]), 0..12),
                    ),
                    0..5,
                ),
                0u8..3,
            ),
            0..12,
        )
    ) {
        let mut s = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 50 50">"#);
        let mut open = Vec::new();
        for (name, attrs, nest) in &elems {
            s.push('<');
            s.push_str(name);
            for (k, v) in attrs {
                s.push(' ');
                s.push_str(k);
                s.push_str("=\"");
                s.push_str(&v.concat());
                s.push('"');
            }
            match nest {
                0 => s.push_str("/>"),
                1 => {
                    s.push('>');
                    open.push(*name);
                }
                _ => {
                    s.push_str("></");
                    s.push_str(name);
                    s.push('>');
                }
            }
        }
        while let Some(n) = open.pop() {
            s.push_str("</");
            s.push_str(n);
            s.push('>');
        }
        s.push_str("</svg>");
        let doc = parse_svg(s.as_bytes());
        prop_assert!(doc.is_ok(), "{s}: {doc:?}");
        let doc = doc.unwrap();
        let mut h = RenderHarness::new(24, 24, ColorFormat::Rgb565);
        h.paint(|pa| doc.render(pa, Rect::new(0, 0, 24, 24)));
        let mut c = RenderHarness::new(24, 24, ColorFormat::Rgb565);
        c.paint_chunked(5, |pa| doc.render(pa, Rect::new(0, 0, 24, 24)));
        prop_assert!(h.data() == c.data());
    }
}
