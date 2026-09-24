# Demo image sources

Every image in this folder is drawn procedurally by `gen_frames.py` (Python 3, standard
library only). No third-party artwork is used; the files are covered by the repository
license (MIT OR Apache-2.0).

| File | Content |
|------|---------|
| `frame0.png` … `frame3.png` | 32 × 32 grey ring with a blue quarter sector, rotated 90° per frame (the `controls` animimg) |
| `power_off.png`, `power_on.png` | 36 × 36 grey / blue disc with a white power sign (the `controls` image button) |

The converted images (`src/assets/*.rs` + `.bin`, ARGB8888) are generated with the commands in
the header of `gen_frames.py`.
