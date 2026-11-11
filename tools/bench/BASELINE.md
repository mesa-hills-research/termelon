# Baseline: ratatui 0.30 and crossterm 0.29

What ratatui and crossterm cost on a terminal coding agent's output workloads, measured before the
framework's own output code exists. The rows ending in `-rec` replay frames recorded from Arko and are
the baseline the targets are set against. The others are generated screens of the same shape.

## Results

```
workload         stack    frames render_us_p50 render_us_p95 encode_us_p50 encode_us_p95 bytes_mean bytes_max allocs alloc_bytes
idle             ratatui     100         117.4         169.3         116.9         173.5      212.6       312    1.0           5
streaming        ratatui     300          47.6          86.2          47.8         100.0      666.4      6141    1.7           8
scrolling        ratatui     100          47.0          78.4          94.4         147.9     4575.7      5590   10.0          50
typing           ratatui     100          47.2          87.2          47.7          64.8       83.2        95    1.0           5
full-redraw      ratatui      50         378.3         438.2        5031.6        5294.3   981685.8    983537    1.0           5
idle-rec         ratatui      99         116.9         167.9         116.4         170.1      220.1       319    2.1          11
streaming-rec    ratatui     299          46.6          84.0          47.9         140.8     1361.6      8349    3.3          17
scrolling-rec    ratatui      99          46.8          82.5          83.0         133.9     3522.8      3807    9.4          47
typing-rec       ratatui      99          47.1          77.0          47.1          87.9       82.1        93    1.0           5
full-redraw-rec  ratatui       6         300.8         354.7         929.6         998.7    35889.3     36909   26.3         132

workload         stack    repeats render_us_min render_us_max render_pct encode_us_min encode_us_max encode_pct
idle             ratatui       10         114.6         122.7        7.1         112.5         122.2        8.7
streaming        ratatui       10          46.6          48.9        5.1          46.6          50.3        7.9
scrolling        ratatui       10          46.5          49.0        5.3          92.1          97.5        6.0
typing           ratatui       10          46.1          48.6        5.5          46.6          49.3        5.8
full-redraw      ratatui       10         367.1         408.7       11.3        4877.7        5219.4        7.0
idle-rec         ratatui       10         114.9         120.6        5.0         113.4         119.8        5.7
streaming-rec    ratatui       10          46.0          48.5        5.4          46.7          50.9        9.1
scrolling-rec    ratatui       10          46.2          48.7        5.5          80.5          85.6        6.3
typing-rec       ratatui       10          45.2          48.7        7.9          45.8          49.3        7.5
full-redraw-rec  ratatui       10         278.6         333.3       19.6         833.2         962.6       15.5
```

The first table pools the frames of all 10 repeats. Times are microseconds per frame, `p50` the median
and `p95` the 95th percentile.

| Column | Meaning |
|---|---|
| `frames` | Timed frames in one repeat. The first recorded frame only sets the screen up. |
| `render_us_*` | Building ratatui's buffer from the neutral frame, a copy of every cell |
| `encode_us_*` | `Terminal::apply_buffer_with_cursor`, which is the diff, `CrosstermBackend::draw`, the cursor, and the buffer swap and reset, plus the synchronized-update commands around it. In the full redraws it also holds the resize before the frame |
| `bytes_*` | Bytes written for one frame, mean and largest |
| `allocs`, `alloc_bytes` | Allocations and bytes allocated in the timed region of one frame, mean |

The second table shows how much the median of one repeat differed from another's in the same run.

| Workload | Screen | Frames |
|---|---|---|
| `idle` | 200 by 60, a turn running, only the spinner, the shimmer over "Working…" and the status timer change | 100 |
| `streaming` | 120 by 40, a reply grows a word a frame at the bottom, the view follows | 300 |
| `scrolling` | 120 by 40, Page Up through a long transcript, a page a frame | 100 |
| `typing` | 120 by 40, a character a frame in the input box | 100 |
| `full-redraw` | 300 by 100, every cell in its own RGB colours, a resize before each frame. This is a worst case for colour changes | 50 |
| `full-redraw-rec` | 300 by 100, Arko's screen, a resize before each frame | 6 |

## What the numbers say

- Both passes cost the size of the screen, not the size of the change. An idle frame at 200 by 60 takes
  117 µs to build and 117 µs to diff and encode (about 10 ns for each of 12,000 cells) and writes 220 bytes.
  A typed character at 120 by 40 takes 47 µs for each pass and writes 82.
- The generated and recorded rows take about the same time to build the buffer, and to diff and encode,
  except in scrolling (94 and 83 µs). They differ in bytes, since Arko's real screens hold more changing
  text. A recorded streaming frame writes 1,362 bytes on average, against 666 generated.
- An idle frame writes 220 bytes, close to the 200 that Arko's own measurement gives.
- Building the buffer allocates nothing. Diff and encode allocates at least once a frame, 5 bytes for the
  closing reset the backend writes after every `draw`, and once more for each text attribute it writes,
  since crossterm builds a `String` for it. Scrolling frames make about 10.
- A frame that changes nothing still writes about 50 bytes, the synchronized update, the closing resets
  and the cursor. Arko's own `CursorBackend` writes nothing for it, and isn't in this baseline.
- A recorded full redraw takes 0.93 ms and 36 KB. The generated one, where every cell changes colour,
  takes 5.0 ms and 982 KB.

## Machine and toolchain

- Intel Core i9-13980HX, 6 vCPUs of a virtual machine shared with other jobs, 22 GiB of memory, Ubuntu
  26.04.1, Linux 7.0.0-38-generic.
- Load average (1, 5 and 15 minutes) before and after the run above: 2.42 3.52 4.32 and 2.47 3.51 4.31.
- rustc 1.97.1 and cargo 1.97.1, with the workspace's release profile (no changes to the defaults).
- ratatui 0.30.2 (ratatui-core 0.1.2, ratatui-crossterm 0.1.2) with default features off and the features
  `crossterm`, `underline-color` and `unstable-backend-writer`, which only opens the backend's writer. crossterm 0.29.0.
- An allocation counter wraps the system allocator, so each allocation costs a few atomic increments. It
  affects every stack alike.

## Command

```
cargo build --release -p bench
target/release/bench --repeat 10 --recordings DIR
```

`DIR` holds `idle.styled`, `streaming.styled`, `scrolling.styled`, `typing.styled` and `redraw.styled`,
frames recorded from Arko in the styled-text format, which are kept outside the repository. Without
`--recordings` only the generated workloads run. `bench --help` lists the rest.

## How noisy it was

Three runs of this command, minutes apart at a load of 1.5 to 2.5, gave medians within 0.8 to 4.4% of each
other, and the run above is the middle one. In the run above the median of one repeat differs from another's by
5 to 20%, and in the other two by up to 54%, so each figure pools 10 repeats. Two earlier runs at a load
of 3.4 to 5.7 were 30 to 55% slower for the same code, so only compare numbers taken in the same quiet
moment. The later comparison with the framework should interleave the stacks in
every repeat, which `bench` does when it has two. Bytes and allocations don't vary.

## Not covered

- What a terminal does with the bytes.
- ratatui's size query before each frame, a system call that `Terminal::draw` makes and this baseline leaves out.
- Building lines into the buffer with the widgets, which needs the framework's own API to compare. Here
  the buffer is a copy of the frame.
- Paste and mouse drags, which are input.
- Hyperlinks, overlines and underline shapes, which ratatui's cell can't hold. The frames here use none
  of them.
