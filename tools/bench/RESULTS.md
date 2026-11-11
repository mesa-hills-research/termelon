# termelon against ratatui 0.30 and crossterm 0.29

termelon's buffer and encoder against the baseline stack, on the workloads of
[BASELINE.md](BASELINE.md), played interleaved in the same run. `termelon` has scroll detection off,
as `Terminal` does today, and `termelon-scroll` has it on. Both encode for a terminal that erases
with the pen's background (`bce`), as every terminal of the known-terminal table does but tmux and
GNU screen. Rows ending in `-rec` replay frames recorded from Arko.

## Against the targets

The targets are those of DESIGN.md section 15.3, judged against ratatui's figures in this run, not
the VM's in BASELINE.md. Figures are medians of 10 repeats.

| Target | Result | Met |
|---|---|---|
| Diff and encode, idle animation frame, under 0.2 of the baseline and 30 µs | 5.3 µs against 79.7 (0.07) recorded, 5.9 against 79.4 generated | Yes |
| The same with a partial frame, under 5 µs | Not in the bench, which has no regions. About 1 µs on a generated 200 by 60 screen | Not measured here |
| Diff and encode, streaming frame, under 0.5 of the baseline | 2.6 µs against 32.4 (0.08) recorded, 4.1 (0.13) with scroll detection | Yes |
| Bytes, streaming frame with scroll detection, under 0.2 of the baseline | 126 against 1,362 (0.09) recorded, 91 against 666 (0.14) generated | Yes |
| Diff and encode, full redraw, under 0.7 of the baseline and 2 ms | 178 µs against 554 (0.32) recorded, 1.32 ms against 2.96 (0.45) generated | Yes |
| Bytes, full redraw, under 0.85 of the baseline | 6,579 against 35,889 (0.18) recorded, 587,344 against 981,686 (0.60) generated | Yes |
| Bytes per Arko animation frame, under 0.8 of today's 200 | 158.5 against 220.1 (0.72) | Yes |
| Allocations per frame once warm (diff and encode) | 0 in every recorded workload. In the generated idle and scrolling ones, one frame allocates 2,960 bytes, the second of the two buffers drawing for the first time, since each workload warms up with one frame | Yes |
| Memory per cell, 16 bytes | 16, checked at compile time | Yes |

The targets for Arko's whole frame after the port, key to bytes, pastes and the startup probe need
the port and the terminal, and aren't measured here. The hand-off to the writer thread isn't in the
bench.

## What the numbers say

- An unchanged row costs a comparison of plain bytes, so an idle or typing frame costs 2 to 6 µs
  whatever the screen holds, where ratatui's diff costs the screen (about 80 µs at 200 by 60).
- Arko paints its theme's background on every cell. A full redraw clears the screen in that
  background and writes only the cells that differ from it, and a row that ends in spaces of one
  background ends with `CSI K`, so a recorded full redraw writes 0.18 of ratatui's bytes. A
  terminal whose erase leaves the default background (tmux, GNU screen, or one the table doesn't
  know) gets those spaces written, as ratatui writes them, at about 0.96 of its bytes.
- Scroll detection takes recorded streaming from 632 bytes a frame to 126, and recorded paging
  from 1,786 to 842, for 1.5 to 3 µs a frame of hashing rows. On the generated paging, where pages
  share fewer rows, it costs 22 µs a frame and saves 3% of the bytes.
- Building the buffer costs 1.3 to 1.5 times ratatui's, and 2.3 times in the generated full redraw,
  where every cell has its own colours. The ratatui stack copies cells into ratatui's buffer field
  by field, and the termelon stack writes them through `set_string` and `cell_mut`, the API a
  program uses, which measures text and keeps wide characters whole. No target covers it.

## The run

```
workload         stack    frames render_us_p50 render_us_p95 encode_us_p50 encode_us_p95 bytes_mean bytes_max allocs alloc_bytes
idle             ratatui     100          81.5          83.0          79.4          81.5      212.6       312    1.0           5
idle             termelon    100         113.6         117.2           5.9           6.5      151.8       213    0.0          30
idle             termelon-scroll    100         113.6         117.0           9.8          10.4      151.8       213    0.0          30
streaming        ratatui     300          32.8          33.2          32.3          64.0      666.4      6141    1.7           8
streaming        termelon    300          50.1          51.2           2.3          33.8      347.6      2972    0.0           0
streaming        termelon-scroll    300          49.9          51.4           3.8          24.7       90.5       193    0.0           0
scrolling        ratatui     100          32.8          33.3          63.7          67.7     4575.7      5590   10.0          50
scrolling        termelon    100          50.0          51.8          30.5          33.3     2408.8      3024    0.0          30
scrolling        termelon-scroll    100          49.9          51.7          52.2          54.8     2341.4      3028    0.0          30
typing           ratatui     100          32.5          32.9          32.0          33.0       83.2        95    1.0           5
typing           termelon    100          50.2          51.6           2.2           2.3       62.8        78    0.0           0
typing           termelon-scroll    100          50.2          51.6           3.8           3.8       62.8        78    0.0           0
full-redraw      ratatui      50         235.2         237.3        2957.4        3224.5   981685.8    983537    1.0           5
full-redraw      termelon     50         538.2         542.6        1323.9        1333.5   587344.2    589506    0.0           0
full-redraw      termelon-scroll     50         538.2         542.7        1324.5        1334.7   587344.2    589506    0.0           0
idle-rec         ratatui      99          81.6          83.0          79.7          81.8      220.1       319    2.1          11
idle-rec         termelon     99         112.0         115.2           5.3           5.9      158.5       218    0.0           0
idle-rec         termelon-scroll     99         111.9         115.5           9.2           9.8      158.5       218    0.0           0
streaming-rec    ratatui     299          32.7          33.3          32.4          91.3     1361.6      8349    3.3          17
streaming-rec    termelon    299          48.7          50.0           2.6          66.8      632.0      3631    0.0           0
streaming-rec    termelon-scroll    299          48.6          49.9           4.1          26.3      126.0       303    0.0           0
scrolling-rec    ratatui      99          32.7          34.0          56.1          57.7     3522.8      3807    9.4          47
scrolling-rec    termelon     99          49.2          50.6          30.1          31.8     1786.2      2043    0.0           0
scrolling-rec    termelon-scroll     99          49.2          50.8          33.2          36.2      842.0      1232    0.0           0
typing-rec       ratatui      99          32.7          34.1          32.2          33.2       82.1        93    1.0           5
typing-rec       termelon     99          49.0          50.5           2.2           2.3       61.3        75    0.0           0
typing-rec       termelon-scroll     99          49.0          50.4           3.8           3.8       61.3        75    0.0           0
full-redraw-rec  ratatui       6         203.4         205.7         554.3         560.6    35889.3     36909   26.3         132
full-redraw-rec  termelon      6         258.2         265.3         177.9         185.8     6579.0      7423    0.0           0
full-redraw-rec  termelon-scroll      6         258.0         266.7         177.2         180.9     6579.0      7423    0.0           0
```

```
workload         stack    repeats render_us_min render_us_max render_pct encode_us_min encode_us_max encode_pct
idle             ratatui       10          81.2          82.0        0.9          78.9          79.8        1.1
idle             termelon      10         111.9         116.6        4.3           5.8           6.0        2.4
idle             termelon-scroll      10         109.9         116.6        6.1           9.7           9.8        0.5
streaming        ratatui       10          32.7          33.0        1.0          32.2          32.3        0.4
streaming        termelon      10          48.4          50.7        4.8           2.3           2.3        0.4
streaming        termelon-scroll      10          47.9          51.1        6.7           3.8           3.8        1.3
scrolling        ratatui       10          32.7          33.0        1.2          63.3          64.0        1.1
scrolling        termelon      10          48.0          51.1        6.5          29.8          30.8        3.2
scrolling        termelon-scroll      10          47.6          51.0        7.3          52.0          52.4        0.7
typing           ratatui       10          32.5          32.6        0.3          31.5          32.4        3.0
typing           termelon      10          48.2          51.3        6.4           2.2           2.2        0.4
typing           termelon-scroll      10          48.2          51.2        6.3           3.8           3.8        0.3
full-redraw      ratatui       10         234.5         235.5        0.4        2931.4        3218.3        9.8
full-redraw      termelon      10         537.7         541.8        0.8        1321.8        1326.1        0.3
full-redraw      termelon-scroll      10         537.6         541.3        0.7        1321.1        1325.7        0.4
idle-rec         ratatui       10          81.3          82.0        0.9          79.0          80.1        1.4
idle-rec         termelon      10         108.5         114.9        5.9           5.3           5.3        0.9
idle-rec         termelon-scroll      10         107.1         115.1        7.5           9.2           9.2        0.2
streaming-rec    ratatui       10          32.6          32.8        0.5          32.3          32.6        0.9
streaming-rec    termelon      10          47.8          49.3        3.3           2.6           2.6        0.8
streaming-rec    termelon-scroll      10          47.9          49.6        3.5           4.1           4.1        0.5
scrolling-rec    ratatui       10          32.7          33.0        0.9          55.9          56.4        0.9
scrolling-rec    termelon      10          48.9          50.3        3.0          30.0          30.2        0.8
scrolling-rec    termelon-scroll      10          49.0          50.3        2.8          33.2          33.3        0.3
typing-rec       ratatui       10          32.7          32.9        0.9          32.0          32.8        2.6
typing-rec       termelon      10          48.7          50.2        3.0           2.2           2.2        0.4
typing-rec       termelon-scroll      10          48.8          50.1        2.7           3.7           3.8        0.5
full-redraw-rec  ratatui       10         202.8         204.7        0.9         552.2         559.6        1.3
full-redraw-rec  termelon      10         256.5         265.0        3.3         176.8         181.3        2.5
full-redraw-rec  termelon-scroll      10         256.8         265.5        3.4         175.8         179.0        1.8
```

The columns are those of BASELINE.md.

## Machine and toolchain

- A whole AMD Ryzen 7 9800X3D machine with 16 threads and 61 GB, rented for benchmarks with no
  other tenant. The bench ran on one core (`taskset -c 3`).
- Load average 4.06 5.77 3.99 before the run, as another job had just ended, and 3.58 5.61 3.96
  after. ratatui's figures in this run are within 1% of those of a run at a load of 2.5, but its
  generated full redraw, which varied between 2.9 and 6.0 ms from run to run.
- rustc 1.97.1, the workspace's release profile, and the crate versions of BASELINE.md.

## Command

```
cargo build --release -p bench
taskset -c 3 target/release/bench --repeat 10 --recordings DIR
```

`DIR` holds the recordings BASELINE.md describes. The stacks take turns within each repeat.
