//! Software display rotation.

use proptest::prelude::*;
use twine_core::Rotation;
use twine_render::rotate_buffer;

proptest! {
    #[test]
    fn rotate_four_times_is_identity(w in 1usize..40, h in 1usize..40, bpp in 1usize..=4, seed in any::<u32>(), r in 1usize..4) {
        let rot = [Rotation::Deg0, Rotation::Deg90, Rotation::Deg180, Rotation::Deg270][r];
        let mut rng = twine_core::XorShift32::new(seed.max(1));
        let src: Vec<u8> = (0..w * h * bpp).map(|_| rng.next_u32() as u8).collect();
        let mut cur = src.clone();
        let (mut cw, mut ch) = (w, h);
        for _ in 0..4 {
            let (nw, nh) = if rot == Rotation::Deg180 { (cw, ch) } else { (ch, cw) };
            let mut dst = vec![0u8; w * h * bpp];
            rotate_buffer(&cur, cw * bpp, &mut dst, nw * bpp, cw, ch, rot, bpp).unwrap();
            cur = dst;
            (cw, ch) = (nw, nh);
        }
        prop_assert_eq!(cur, src);
    }
}
