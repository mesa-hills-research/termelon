# tuiframe against ratatui 0.30 and crossterm 0.29

tuiframe's buffer and encoder against the baseline stack, on the workloads of
[BASELINE.md](BASELINE.md), played interleaved in the same run. `tuiframe` has scroll detection off,
as `Terminal` does today, and `tuiframe-scroll` has it on. Rows ending in `-rec` replay frames
recorded from Arko.

## Against the targets

The targets are those of DESIGN.md section 15.3, judged against ratatui's figures in this run, not
the VM's in BASELINE.md. Figures are medians of 10 repeats.

| Target | Result | Met |
|---|---|---|
| Diff and encode, idle animation frame, under 0.2 of the baseline and 30 µs | 6.0 µs against 79.3 (0.08) recorded, 6.5 against 78.9 generated | Yes |
| The same with a partial frame, under 5 µs | Not in the bench, which has no regions. About 1 µs on a generated 200 by 60 screen | Not measured here |
| Diff and encode, streaming frame, under 0.5 of the baseline | 2.7 µs against 32.3 (0.08) recorded, 4.7 (0.15) with scroll detection | Yes |
| Bytes, streaming frame with scroll detection, under 0.2 of the baseline | 148 against 1,362 (0.11) recorded, 108 against 666 (0.16) generated | Yes |
| Diff and encode, full redraw, under 0.7 of the baseline and 2 ms | 242 µs against 552 (0.44) recorded, 1.25 ms against 5.96 (0.21) generated | Yes |
| Bytes, full redraw, under 0.85 of the baseline | 34,372 against 35,889 (0.96) recorded, 588,079 against 981,686 (0.60) generated | Generated only |
| Bytes per Arko animation frame, under 0.8 of today's 200 | 158.5 against 220.1 (0.72) | Yes |
| Allocations per frame once warm (diff and encode) | 0 in every recorded workload. In the generated idle and scrolling ones, one frame allocates 2,960 bytes, the second of the two buffers drawing for the first time, since each workload warms up with one frame | Yes |
| Memory per cell, 16 bytes | 16, checked at compile time | Yes |

The targets for Arko's whole frame after the port, key to bytes, pastes and the startup probe need
the port and the terminal, and aren't measured here. The hand-off to the writer thread isn't in the
bench.

## What the numbers say

- An unchanged row costs a comparison of plain bytes, so an idle or typing frame costs 2 to 7 µs
  whatever the screen holds, where ratatui's diff costs the screen (about 80 µs at 200 by 60).
- A recorded full redraw writes nearly as many bytes as ratatui's, 0.96 of them. Arko paints its
  theme's background on every cell, so every cell goes out, and a space in a background colour is
  written as a space, since erasing with it (`CSI K`, `CSI n X`) fills with the background only in
  terminals with BCE. Where the probe finds BCE, erasing would meet the target.
- Scroll detection takes recorded streaming from 791 bytes a frame to 148, and costs about 2 µs a
  frame for hashing rows. Paging through a transcript (`scrolling`) changes most rows without a
  shift that pays, so detection there costs 11 to 22 µs, saves 16% of the bytes on the recorded
  frames and writes 3% more on the generated ones.
- Building the buffer costs 1.2 to 1.5 times ratatui's, and 2.3 times in the generated full redraw,
  where every cell has its own colours. The ratatui stack copies cells into ratatui's buffer field
  by field, and the tuiframe stack writes them through `set_string` and `cell_mut`, the API a
  program uses, which measures text and keeps wide characters whole. No target covers it.
- The p95 of the full redraws is about 3 ms above the median for both stacks alike, in every run,
  so it's the machine, not either stack.

## The run

```
workload         stack    frames render_us_p50 render_us_p95 encode_us_p50 encode_us_p95 bytes_mean bytes_max allocs alloc_bytes
idle             ratatui     100          81.6          83.3          78.9          81.2      212.6       312    1.0           5
idle             tuiframe    100         110.9         115.7           6.5           7.2      151.8       213    0.0          30
idle             tuiframe-scroll    100         110.7         116.5          11.4          12.2      151.8       213    0.0          30
streaming        ratatui     300          32.8          33.3          32.2          64.6      666.4      6141    1.7           8
streaming        tuiframe    300          49.2          50.6           2.5          32.7      440.1      3666    0.0           0
streaming        tuiframe-scroll    300          49.0          51.0           4.5          26.4      107.5       435    0.0           0
scrolling        ratatui     100          32.8          33.4          63.9          68.3     4575.7      5590   10.0          50
scrolling        tuiframe    100          49.0          50.9          29.3          32.6     3301.2      3893    0.0          30
scrolling        tuiframe-scroll    100          48.9          51.2          51.9          55.7     3414.0      4957    0.0          30
typing           ratatui     100          32.6          33.9          32.1          33.0       83.2        95    1.0           5
typing           tuiframe    100          49.5          51.0           2.5           2.6       62.8        78    0.0           0
typing           tuiframe-scroll    100          49.1          51.2           4.4           4.6       62.8        78    0.0           0
full-redraw      ratatui      50         234.0        3235.9        5962.1        6108.3   981685.8    983537    1.0           5
full-redraw      tuiframe     50         539.5        3541.0        1250.7        4250.8   588078.8    589930    0.0           0
full-redraw      tuiframe-scroll     50         539.6        3541.7        1252.7        4253.0   588078.8    589930    0.0           0
idle-rec         ratatui      99          81.6          83.3          79.3          81.8      220.1       319    2.1          11
idle-rec         tuiframe     99         109.0         114.0           6.0           6.6      158.5       218    0.0           0
idle-rec         tuiframe-scroll     99         109.2         114.1          11.0          11.7      158.5       218    0.0           0
streaming-rec    ratatui     299          32.7          34.0          32.3          91.2     1361.6      8349    3.3          17
streaming-rec    tuiframe    299          47.8          49.4           2.7          64.7      790.6      4649    0.0           0
streaming-rec    tuiframe-scroll    299          47.7          49.3           4.7          27.6      148.0       417    0.0           0
scrolling-rec    ratatui      99          32.7          34.0          56.0          58.0     3522.8      3807    9.4          47
scrolling-rec    tuiframe     99          47.8          49.7          26.6          28.0     2412.0      2584    0.0           0
scrolling-rec    tuiframe-scroll     99          47.9          49.4          38.0          41.3     2034.9      2920    0.0           0
typing-rec       ratatui      99          32.7          34.0          32.2          33.2       82.1        93    1.0           5
typing-rec       tuiframe     99          47.6          49.2           2.4           2.5       61.3        75    0.0           0
typing-rec       tuiframe-scroll     99          47.7          49.0           4.4           4.5       61.3        75    0.0           0
full-redraw-rec  ratatui       6         202.9        3205.3         551.6        3554.7    35889.3     36909   26.3         132
full-redraw-rec  tuiframe      6         247.3         255.9         241.8        3246.3    34371.7     34967    0.0           0
full-redraw-rec  tuiframe-scroll      6         248.6         254.1         242.0        3244.2    34371.7     34967    0.0           0
```

```
workload         stack    repeats render_us_min render_us_max render_pct encode_us_min encode_us_max encode_pct
idle             ratatui       10          81.2          82.1        1.1          77.7          79.6        2.5
idle             tuiframe      10         109.3         114.3        4.6           6.4           6.6        3.1
idle             tuiframe-scroll      10         108.4         115.7        6.7          11.3          11.5        1.5
streaming        ratatui       10          32.6          33.1        1.3          31.8          32.3        1.5
streaming        tuiframe      10          48.4          50.0        3.3           2.5           2.5        2.5
streaming        tuiframe-scroll      10          48.4          50.6        4.5           4.4           4.5        1.8
scrolling        ratatui       10          32.7          33.2        1.6          63.6          64.5        1.4
scrolling        tuiframe      10          48.2          49.8        3.3          28.8          30.0        4.0
scrolling        tuiframe-scroll      10          48.3          50.4        4.4          51.5          52.9        2.7
typing           ratatui       10          32.5          33.0        1.4          31.5          32.3        2.4
typing           tuiframe      10          48.7          50.3        3.3           2.4           2.5        3.3
typing           tuiframe-scroll      10          48.7          50.9        4.6           4.4           4.4        1.8
full-redraw      ratatui       10         233.0         234.3        0.6        5955.5        6000.6        0.8
full-redraw      tuiframe      10         538.4         541.0        0.5        1241.6        1256.5        1.2
full-redraw      tuiframe-scroll      10         538.6         541.3        0.5        1241.8        1259.3        1.4
idle-rec         ratatui       10          81.3          82.0        0.8          79.1          79.6        0.7
idle-rec         tuiframe      10         107.0         112.5        5.2           6.0           6.0        1.3
idle-rec         tuiframe-scroll      10         107.1         112.5        5.0          11.0          11.1        0.6
streaming-rec    ratatui       10          32.6          32.9        0.7          32.3          32.4        0.5
streaming-rec    tuiframe      10          46.5          48.6        4.5           2.7           2.7        1.1
streaming-rec    tuiframe-scroll      10          46.6          48.4        4.0           4.6           4.7        1.3
scrolling-rec    ratatui       10          32.7          32.8        0.5          55.9          56.2        0.6
scrolling-rec    tuiframe      10          47.4          49.2        3.7          26.2          26.9        2.7
scrolling-rec    tuiframe-scroll      10          47.6          48.5        2.0          37.6          38.4        2.3
typing-rec       ratatui       10          32.7          32.8        0.5          31.6          32.4        2.4
typing-rec       tuiframe      10          47.3          49.0        3.6           2.4           2.4        1.7
typing-rec       tuiframe-scroll      10          47.4          48.4        2.1           4.3           4.4        1.2
full-redraw-rec  ratatui       10         202.6         203.4        0.4         548.4         557.5        1.7
full-redraw-rec  tuiframe      10         246.0         255.5        3.9         240.9         243.6        1.1
full-redraw-rec  tuiframe-scroll      10         245.9         252.1        2.5         240.3         246.7        2.7
```

The columns are those of BASELINE.md.

## Machine and toolchain

- A whole AMD Ryzen 7 9800X3D machine with 16 threads and 61 GB, rented for benchmarks with no
  other tenant. The bench ran on one core (`taskset -c 3`).
- Load average 3.04 2.53 1.72 before the run and 2.81 2.50 1.73 after.
- rustc 1.97.1, the workspace's release profile, and the crate versions of BASELINE.md.

## Command

```
cargo build --release -p bench
taskset -c 3 target/release/bench --repeat 10 --recordings DIR
```

`DIR` holds the recordings BASELINE.md describes. The stacks take turns within each repeat.
