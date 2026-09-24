//! Loading: the JSON model, animatable forms, legacy keyframes, unsupported features, precomp
//! references and invalid input.
#![allow(clippy::float_cmp, clippy::cast_precision_loss)] // exact float values are expected; small ints become floats

use twine_lottie::model::{Animatable, LayerKind, Position, Shape};
use twine_lottie::{LottieError, load};

fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// Every sample file in `tests/data`.
fn samples() -> Vec<(String, Vec<u8>)> {
    let dir = format!("{}/tests/data", env!("CARGO_MANIFEST_DIR"));
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&p).unwrap(),
            )
        })
        .collect();
    v.sort();
    v
}

#[test]
fn load_minimal_shape_layer() {
    let json = br#"{"fr":24,"ip":0,"op":48,"w":64,"h":32,"layers":[
        {"ty":4,"ind":7,"nm":"dot","ip":0,"op":48,"st":0,"ks":{"p":{"a":0,"k":[32,16]}},
         "shapes":[{"ty":"el","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[10,10]}},
                   {"ty":"fl","c":{"a":0,"k":[1,0.5,0,1]},"o":{"a":0,"k":80},"r":2}]}]}"#;
    let comp = load(json).unwrap();
    assert_eq!(
        (comp.w, comp.h, comp.fr, comp.ip, comp.op),
        (64.0, 32.0, 24.0, 0.0, 48.0)
    );
    assert_eq!(comp.frame_count(), 48.0);
    assert_eq!(comp.duration_secs(), 2.0);
    let l = &comp.layers[0];
    assert_eq!(l.ty, LayerKind::Shape);
    assert_eq!((l.ind, l.name.as_str()), (Some(7), "dot"));
    assert_eq!(
        l.ks.position,
        Position::Combined(Animatable::Static([32.0, 16.0]))
    );
    assert_eq!(l.shapes.len(), 2);
    let Shape::Fill(f) = &l.shapes[1] else {
        panic!("fill expected")
    };
    assert_eq!(f.c, Animatable::Static([1.0, 0.5, 0.0, 1.0]));
    assert_eq!(f.o, Animatable::Static(80.0));
    assert_eq!(f.rule, twine_lottie::model::FillRule::EvenOdd);
    assert!(comp.unsupported().is_empty());
}

#[test]
fn animatable_static_and_keyframed_forms() {
    // `{"a":0,"k":n}`, `{"k":[n]}`, bare number, `{"a":1,"k":[…]}` and keyframes without `a`.
    let json = br#"{"fr":30,"ip":0,"op":30,"w":10,"h":10,"layers":[{"ty":4,"ks":{
        "o":{"a":0,"k":50},
        "r":{"k":[45]},
        "s":{"a":1,"k":[{"t":0,"s":[100,100]},{"t":10,"s":[50,200]}]},
        "a":{"k":[{"t":5,"s":[1,2],"h":1},{"t":9,"s":[3,4]}]},
        "p":{"s":true,"x":{"a":0,"k":7},"y":{"a":1,"k":[{"t":0,"s":[0]},{"t":10,"s":[10]}]}}
      },"shapes":[{"ty":"st","c":{"k":[0,0,0,1]},"o":{"k":100},"w":3,"lc":1,"lj":3,"ml":10}]}]}"#;
    let comp = load(json).unwrap();
    let ks = &comp.layers[0].ks;
    assert_eq!(ks.opacity, Animatable::Static(50.0));
    assert_eq!(ks.rotation, Animatable::Static(45.0));
    let Animatable::Keyframed(s) = &ks.scale else {
        panic!("scale keyframed")
    };
    assert_eq!(s.frames.len(), 2);
    assert_eq!(s.frames[1].s, [50.0, 200.0]);
    let Animatable::Keyframed(a) = &ks.anchor else {
        panic!("anchor keyframed")
    };
    assert!(a.frames[0].hold);
    let Position::Split(x, y) = &ks.position else {
        panic!("split position")
    };
    assert_eq!(*x, Animatable::Static(7.0));
    assert!(y.is_animated());
    let Shape::Stroke(st) = &comp.layers[0].shapes[0] else {
        panic!("stroke")
    };
    assert_eq!(st.style.w, Animatable::Static(3.0)); // bare number
    assert_eq!(st.style.lc, twine_lottie::model::LineCap::Butt);
    assert_eq!(st.style.lj, twine_lottie::model::LineJoin::Bevel);
    assert_eq!(st.style.ml, 10.0);
}

#[test]
fn legacy_e_values_supported() {
    let comp = load(&data("legacy.json")).unwrap();
    let Position::Combined(Animatable::Keyframed(p)) = &comp.layers[0].ks.position else {
        panic!("keyframed position");
    };
    assert_eq!(p.frames.len(), 2);
    assert_eq!(p.frames[0].e, Some([80.0, 50.0]));
    // The trailing keyframe with only `t` holds the previous end value.
    assert_eq!(p.frames[1].s, [80.0, 50.0]);
    assert_eq!(p.frames[0].to, Some([10.0, 0.0]));
    // Halfway along the spatial curve (linear easing handles).
    let v = comp.layers[0].ks.position.clone();
    let Position::Combined(v) = v else { unreachable!() };
    assert_eq!(v.value(0.0), [20.0, 50.0]);
    assert_eq!(v.value(40.0), [80.0, 50.0]);
    let mid = v.value(20.0);
    assert!(
        (mid[0] - 50.0).abs() < 1e-3 && (mid[1] - 50.0).abs() < 1e-3,
        "{mid:?}"
    );
}

#[test]
fn unsupported_shape_recorded_not_error() {
    let comp = load(&data("unsupported.json")).unwrap();
    let u = comp.unsupported();
    for what in ["luma matte", "rounded corners", "effects", "text layer"] {
        assert_eq!(u.iter().filter(|w| *w == what).count(), 1, "{what}: {u:?}");
    }
    let Shape::Group(g) = &comp.layers[1].shapes[0] else {
        panic!("group")
    };
    assert!(matches!(&g.items[2], Shape::Unsupported(t) if t == "rd"));
    assert_eq!(comp.layers[4].ty, LayerKind::Unsupported(5));
}

#[test]
fn precomp_reference_resolved() {
    let comp = load(&data("precomp.json")).unwrap();
    assert_eq!(comp.assets.len(), 1);
    assert_eq!(comp.assets[0].id, "comp_0");
    let l = &comp.layers[0];
    assert_eq!(l.ty, LayerKind::Precomp);
    assert_eq!(l.ref_id.as_deref(), Some("comp_0"));
    assert_eq!(l.asset, Some(0));
    assert!(l.tm.as_ref().is_some_and(Animatable::is_animated));
    // A dangling reference is reported, not an error.
    let json = br#"{"fr":30,"ip":0,"op":30,"w":10,"h":10,"layers":[{"ty":0,"refId":"nope","ks":{}}]}"#;
    let comp = load(json).unwrap();
    assert_eq!(comp.layers[0].asset, None);
    assert!(comp.unsupported().iter().any(|w| w == "missing precomp asset"));
}

#[test]
fn parents_and_mattes_resolved() {
    let comp = load(&data("parenting.json")).unwrap();
    assert_eq!(comp.layers[0].parent_index, Some(1));
    assert_eq!(comp.layers[1].parent_index, Some(2));
    assert_eq!(comp.layers[2].parent_index, None);
    let comp = load(&data("matte.json")).unwrap();
    assert_eq!(comp.layers[1].matte_index, Some(0));
    assert_eq!(comp.layers[3].matte_index, Some(2));
}

#[test]
fn invalid_json_returns_error() {
    assert!(matches!(load(b""), Err(LottieError::Json { .. })));
    assert!(matches!(load(b"{\"w\": 10,"), Err(LottieError::Json { .. })));
    assert_eq!(load(b"null"), Err(LottieError::Invalid("root is not an object")));
    assert_eq!(load(b"{}"), Err(LottieError::Invalid("missing width `w`")));
    assert!(matches!(
        load(br#"{"w":0,"h":10,"fr":30,"op":10}"#),
        Err(LottieError::Invalid(_))
    ));
    assert!(matches!(
        load(br#"{"w":10,"h":10,"fr":0,"op":10}"#),
        Err(LottieError::Invalid(_))
    ));
    assert!(matches!(
        load(br#"{"w":10,"h":10,"fr":30,"ip":5,"op":5}"#),
        Err(LottieError::Invalid(_))
    ));
    // Deep nesting is rejected by the JSON parser instead of overflowing the stack.
    let deep = "[".repeat(100_000);
    assert!(load(deep.as_bytes()).is_err());
    // Wrong types everywhere are tolerated.
    let json = br#"{"w":10,"h":10,"fr":30,"op":10,"layers":[1,"x",{"ty":"4","ks":7,"shapes":{"a":1},
        "masksProperties":[{"pt":5}]},{"ty":4,"shapes":[{"ty":"gr","it":[{"ty":"fl","c":"red"}]},{"ty":7}]}]}"#;
    let comp = load(json).unwrap();
    assert_eq!(comp.layers.len(), 2);
    let e = LottieError::Json { line: 1, column: 2 };
    assert_eq!(e.to_string(), "invalid JSON at line 1, column 2");
}

#[test]
fn all_samples_load() {
    let files = samples();
    assert!(files.len() >= 15);
    for (name, bytes) in files {
        let comp = load(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!comp.layers.is_empty(), "{name}");
        if name != "unsupported.json" {
            assert!(comp.unsupported().is_empty(), "{name}: {:?}", comp.unsupported());
        }
    }
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2000))]

    /// Arbitrary bytes never panic.
    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..512)) {
        let _ = load(&bytes);
    }

    /// Mutated sample files never panic (byte flips keep most of the structure intact).
    #[test]
    fn mutated_samples_never_panic(idx in 0usize..64, flips in proptest::collection::vec((0usize..1_000_000, proptest::prelude::any::<u8>()), 1..8)) {
        let files = samples();
        let (_, mut bytes) = files[idx % files.len()].clone();
        for (pos, b) in flips {
            let n = bytes.len();
            bytes[pos % n] = b;
        }
        let _ = load(&bytes);
    }
}
