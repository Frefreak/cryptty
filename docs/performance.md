# Terminal optimization sequence

The optimization work on October 15 and 16, 2026 moved from reducing
allocation costs to changing row storage and batching character writes.
The sequence below records what changed, the technique used, and the
measured performance at each checkpoint.

## How to read the measurements

Times cover 1,048,576 lines of terminal output, ending at the cursor-position
reply after all output. Lower times indicate higher processing throughput.
Both terminals used native Wayland, Intel UHD Graphics 770, 17 pt monospace
fonts, an 80 by 21 grid, and 10,000 history rows. Cryptty used release builds.

The initial and intermediate checkpoints used three trials and a 0.5-second
warmup. The last two checkpoints used seven alternating-order trials and a
two-second warmup, with grid checks before and after timing. A startup scale
change prompted the stronger checks. These sequential runs show progression;
run variation and grouped changes limit attribution to individual edits.
Frame presentation requires separate timing.

## 1 Reuse allocations and shrink cells

`perf` initially attributed 30% of CPU samples to dropping rows, 13% to cell
vector initialization, and 36% to character placement. Recycled the oldest
history row at the 10,000-row limit, keeping its allocation for the new row.
Moved optional combining strings behind `Box<String>`, shared hyperlinks with
`Arc<String>`, and stored RGB channels as three bytes. Together these changes
reduced each cell from 88 to 56 bytes. A guarded ASCII placement path also
avoided general placement work for ordinary narrow cells.

Plain output changed from **1.237 to 0.601 s**, colors from **1.280 to
0.664 s**, Unicode from **1.081 to 0.755 s**, and links from **0.952 to
0.563 s**. Plain-output peak RSS fell from about 185.5 to 161.0 MiB.
This checkpoint became commit `e423f51`.

## 2 Change row storage

Profiling `e423f51` still placed 44% of plain-output CPU samples in scrolling.
Replaced the visible `Vec<Row>` with `VecDeque<Row>` for ring rotation during
full-screen scrolling. Represented each row as a written prefix plus an
implicit blank suffix, allowing constant-time logical clearing. Materialize
blanks for slice edits, and make indexing, iteration, equality, and cloning
respect logical contents.

At this storage checkpoint, plain output measured **0.944 s**, compared with
**0.601 s** previously; Unicode measured **0.889 s**, compared with
**0.755 s**. A fresh profile placed scrolling near 2% and character printing
near 82% of CPU samples. The next step addressed temporary cell copying in
that newly dominant write path.

## 3 Write text cell fields directly

Instruction-level inspection with `perf annotate` highlighted temporary
cell copies and stack traffic. Updated content, style, and hyperlink fields
directly in the destination cell, after preparing any gap before it.

Plain output changed from **0.944 to 0.494 s**, colors from **0.965 to
0.578 s**, and Unicode from **0.889 to 0.635 s**. This retained the row
redesign while reducing its per-character write cost.

## 4 Write wide continuation cells directly

Applied the same direct-field approach to the continuation cell that follows
a wide character. Borrowed the destination row once for the text cell and its
continuation, preserving style and hyperlink metadata on both cells.

Unicode output changed from **0.635 to 0.494 s**. Plain output stayed near
**0.492 s**; Alacritty measured **0.492 s** in that comparison.

## 5 Coalesce PTY output wakeups

Used a shared `AtomicBool` to coalesce notifications from the PTY reader. The
UI clears the pending flag before draining output, processes at most 32
events, and schedules another batch when it reaches that limit. Keep input
writer wakeups independent so blocked input can resume.

The next Unicode checkpoint measured **0.479 s**, compared with **0.494 s**
previously. Plain output measured **0.499 s**, compared with **0.492 s**.
These small deltas carry more run variation than the larger structural gains.

## 6 Batch ASCII writes

Wrapped the existing `vte` performer with a 256-byte printable-ASCII buffer.
Flush before control callbacks and at input chunk boundaries. Write row
slices in batches, update cursor and prefix bounds once per batch, skip
empty flushes, and skip scanning an already implicit blank suffix. Keep
wrapping, insertion, graphics charsets, and wide-cell overwrites on the
general path.

Plain output moved from the **0.499 s** checkpoint to **0.346 s**, colors
from **0.578 to 0.421 s**, and links from **0.399 to 0.358 s**. Unicode
measured **0.455 s**, alongside Alacritty's **0.458 s**. That near-parity
Unicode result motivated a dedicated append path.

## 7 Append Unicode into the blank suffix

Used the row's prefix bound to recognize an untouched destination. For
positive-width characters away from wrapping and insertion, write the text
and optional continuation directly, then advance the cursor. Existing
placement handles combining marks, boundaries, and overwrites.

The final seven-trial Unicode median changed from **0.455 to 0.411 s**;
Alacritty measured **0.449 s**. The same final build produced:

| Workload | Cryptty median | Alacritty median | Throughput ratio |
| --- | ---: | ---: | ---: |
| Plain ASCII | 0.361 s | 0.455 s | 1.26x |
| SGR colors | 0.426 s | 0.567 s | 1.33x |
| Mixed Unicode and combining text | 0.411 s | 0.449 s | 1.09x |
| OSC 8 links | 0.365 s | 0.401 s | 1.10x |

Ratios divide Alacritty's median time by Cryptty's. Cryptty finished faster
in seven of seven paired trials for plain text, colors, and links, and six
of seven for Unicode. Startup-inclusive CPU time remained higher for
Cryptty. Final peak RSS ranged from about 159 to 160 MiB, versus 190 to
192 MiB for Alacritty 0.17.0 (`94e7c887`). These results apply to the measured
workloads on this machine.

## Measurement records

All times below use seconds and the checkpoint medians described above.

| Checkpoint | Plain | Colors | Unicode | Links |
| --- | ---: | ---: | ---: | ---: |
| Original baseline | 1.237 | 1.280 | 1.081 | 0.952 |
| Compact cells and reuse | 0.601 | 0.664 | 0.755 | 0.563 |
| Lazy rows and ring buffer | 0.944 | 0.965 | 0.889 | 0.439 |
| Direct text writes | 0.494 | 0.578 | 0.635 | 0.403 |
| Direct continuation writes | 0.492 | 0.579 | 0.494 | 0.388 |
| Coalesced wakeups | 0.499 | 0.578 | 0.479 | 0.399 |
| Batched ASCII | 0.346 | 0.421 | 0.455 | 0.358 |
| Unicode append path | 0.361 | 0.426 | 0.411 | 0.365 |

[Stage measurements](benchmarks/2026-10-16-stages.json) retain initial raw
comparisons, intermediate aggregate measurements, and the ASCII checkpoint
runs. [Final raw runs and metadata](benchmarks/2026-10-16.json) retain all 56
final runs, build hashes, and aggregate results.

## Validation and daily use

The final build passed 114 default tests, plus interactive fish, tmux, and
Neovim tests and the GPU/software pixel comparison. Formatting, Clippy
with warnings denied, and independent code reviews also passed. Added
regressions compare buffered and direct parser callbacks, logical row
cloning and materialization, and PTY wakeups across idle and bounded batches.

Daily use can now exercise long-running tmux sessions, Neovim redraws,
resizing, Unicode selection, links, and IME composition. Useful issue
reports include the command or application, renderer and adapter, font
settings, window size, and a short reproducing sequence. macOS runtime and
GPU-free VM throughput remain separate validation work.
