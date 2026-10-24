//! The RLE encoder of font generators (`encode`, feature `std`) is the exact inverse of the
//! glyph decoder at every bit depth: hand-picked run lengths around the format's boundaries
//! (11 repeat bits, 6-bit counts) and random run-heavy data.

use proptest::prelude::*;
use twine_text::Bpp;
use twine_text::encode::{rle_decode, rle_encode};

fn check_roundtrip(values: &[u8], bpp: Bpp) {
    let enc = rle_encode(values, bpp);
    assert_eq!(rle_decode(&enc, bpp, values.len()), values, "{bpp:?}");
}

#[test]
fn rle_roundtrip_runs_of_exactly_11_12_63_64_200() {
    for bpp in Bpp::ALL {
        let max = bpp.max_value();
        for run in [1usize, 2, 3, 11, 12, 13, 62, 63, 64, 65, 74, 75, 76, 200] {
            let mut v = vec![0u8; 3];
            v.extend(std::iter::repeat_n(max, run));
            check_roundtrip(&v, bpp);
            v.push(1);
            check_roundtrip(&v, bpp);
            v.extend(std::iter::repeat_n(0, run));
            check_roundtrip(&v, bpp);
        }
    }
}

fn biased_values() -> impl Strategy<Value = (Bpp, Vec<u8>)> {
    prop::sample::select(Bpp::ALL.to_vec()).prop_flat_map(|bpp| {
        let max = bpp.max_value();
        // Runs of random length (often long) of random values.
        let runs = prop::collection::vec((0..=max, prop_oneof![1usize..4, 1usize..80, 60usize..140]), 1..40);
        runs.prop_map(move |rs| {
            let mut v: Vec<u8> = rs
                .into_iter()
                .flat_map(|(x, n)| std::iter::repeat_n(x, n))
                .collect();
            v.truncate(2000);
            (bpp, v)
        })
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("PROPTEST_CASES").ok().and_then(|s| s.parse().ok()).unwrap_or(1000)))]
    #[test]
    fn rle_roundtrip_random((bpp, values) in biased_values()) {
        prop_assert_eq!(rle_decode(&rle_encode(&values, bpp), bpp, values.len()), values);
    }
}
