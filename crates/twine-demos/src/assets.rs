//! Images of the demos (ARGB8888), converted by `build.rs` from the PNGs in `assets/` (drawn
//! by `assets/gen_frames.py`).

// `FRAME0` … `FRAME3`: the four frames of the `controls` image animation.
include!(concat!(env!("OUT_DIR"), "/frame0.rs"));
include!(concat!(env!("OUT_DIR"), "/frame1.rs"));
include!(concat!(env!("OUT_DIR"), "/frame2.rs"));
include!(concat!(env!("OUT_DIR"), "/frame3.rs"));
// `POWER_OFF`, `POWER_ON`: the `controls` image button.
include!(concat!(env!("OUT_DIR"), "/power_off.rs"));
include!(concat!(env!("OUT_DIR"), "/power_on.rs"));
