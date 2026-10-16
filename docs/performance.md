# Terminal optimization sequence

The optimization work on October 15 and 16, 2026 moved from reducing
allocation costs through row storage, compact cells, and GPU scheduling.
The sequence below records what changed, the technique used, and the
measured performance at each checkpoint.

## How to read the measurements

Stages 1–7 cover 1,048,576 output lines, ending at the cursor-position
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

## First-pass validation and daily use

The first-pass build passed 114 default tests, plus interactive fish, tmux, and
Neovim tests and the GPU/software pixel comparison. Formatting, Clippy
with warnings denied, and independent code reviews also passed. Added
regressions compare buffered and direct parser callbacks, logical row
cloning and materialization, and PTY wakeups across idle and bounded batches.

Daily use can now exercise long-running tmux sessions, Neovim redraws,
resizing, Unicode selection, links, and IME composition. Useful issue
reports include the command or application, renderer and adapter, font
settings, window size, and a short reproducing sequence. macOS runtime and
GPU-free VM throughput remain separate validation work.

## 8 Recycle PTY byte buffers

A fresh profile of `e5031f9` placed 70.75% of plain-output samples in batched
ASCII placement. Unicode printing accounted for 50.78% of Unicode samples.
The experiments below use fresh controls as host load changes. Each
throughput run in stages 8–17 processes 4,194,304 lines.

Tried a bounded pool of 8 KiB buffers returned after consumption, replacing
per-read output allocation with reuse and a recycle channel. Three-trial
medians changed from **1.394 to 1.422 s** for plain output and **1.592 to
1.628 s** for Unicode. Kept the preceding buffer ownership after this
comparison.

## 9 Share styles through an interner

Tried a bounded style cache with shared `Rc<Style>` values. Plain output
changed from **1.735 to 1.523 s**, while colors changed from **2.081 to
2.592 s**, Unicode from **1.863 to 1.980 s**, links from **1.676 to
1.739 s**, and RGB from **2.102 to 2.780 s**.

An `Arc` variant preserved thread-safe ownership, skipped pointer-identical
assignments, and avoided rebuilding unchanged blank styles. Plain output
changed from **2.005 to 1.657 s**, colors from **2.286 to 2.412 s**, and RGB
from **2.350 to 4.528 s**. Kept value styles; the unique-color workload
exposed a large cost in both shared representations.

## 10 Train PGO on the preceding layout

Trained profile-guided optimization on mixed logs, SGR, Unicode, OSC 8,
truecolor, erasure, and paced screen redraws, covering GPU and software
rendering. Timed payloads use different text. Three-trial medians changed
from **2.037 to 2.032 s** for plain output, **2.313 to 2.276 s** for colors,
**2.147 to 2.215 s** for Unicode, **1.843 to 1.752 s** for links, **2.367 to
2.368 s** for RGB, and **7.820 to 7.450 s** for edits. Kept the portable
build while testing structural changes.

## 11 Store width inside text metadata

Moved narrow/wide classification into a private `Character` enum within
`CellText`. This lets Rust reuse an unused enum discriminant for
`Content::Continuation`. Content storage fell from 24 to 16 bytes, and each
cell fell from 56 to 48 bytes on this 64-bit target. Combining strings and
hyperlinks retain their existing ownership.

Three-trial comparisons over 4,194,304 lines changed plain output from
**2.053 to 1.779 s**, colors from **2.323 to 2.069 s**, Unicode from
**2.158 to 2.095 s**, links from **1.800 to 1.692 s**, RGB from **2.338 to
2.167 s**, and edits from **7.960 to 5.671 s**. Plain-output peak RSS fell
from about 148.7 to 142.7 MiB.

## 12 Erase contiguous ranges

CSI K, CSI X, and display erasure now clear ranges through the row's logical
prefix. Full-row erasure resets its prefix; suffix erasure replaces its
blank template after preserving any retained gap. Partial interior erasure
fills a slice. Range expansion preserves wide-character pairs, while the
original requested endpoint controls wrap metadata.

The edit median changed from **5.692 to 1.823 s** against the preceding
build. Other controls stayed close, with links changing from **1.721 to
1.781 s**. Differential tests compare every erase range up to 12 columns
against the previous cell-by-cell operation, including styled blanks,
combining text, links, stale recycled storage, and later writes.

## 13 Pack boolean style attributes

Stored seven boolean attributes in a private byte while retaining typed
colors, blink, and underline values. Cells store `CellStyle` by value.
`Cell::style()` exposes the decoded public `Style`; SGR decodes once,
applies its parameters, and encodes once. The cell footprint fell again,
from 48 to 40 bytes. No style interning or reference counting accompanies
this representation.

Against the preceding build, three-trial medians changed from **1.790 to
1.573 s** for plain output, **2.083 to 1.912 s** for colors, **2.090 to
1.891 s** for Unicode, **1.820 to 1.711 s** for links, **2.121 to 1.968 s**
for RGB, and **1.787 to 1.685 s** for edits. Plain-output peak RSS fell
from about 142.5 to 136.5 MiB. A round-trip test covers all 2,304 boolean,
blink, and underline combinations.

## 14 Try host-specific code generation

Added `-Ctarget-cpu=native` to a separate build after the layout changes.
Three-trial medians changed from **1.617 to 1.596 s** for plain output,
**1.999 to 1.957 s** for colors, **1.931 to 1.883 s** for Unicode, **1.717 to
1.781 s** for links, **2.045 to 2.011 s** for RGB, and **1.692 to 1.720 s**
for edits. Kept portable flags after these small mixed changes.

## 15 Retrain PGO on the compact layout

Retrained from fresh profiles with the same mixed training procedure.
Three-trial medians changed from **1.582 to 1.544 s** for plain output,
**1.892 to 1.810 s** for colors, **1.912 to 1.866 s** for Unicode, **1.713 to
1.597 s** for links, **1.982 to 1.886 s** for RGB, and **1.705 to 1.593 s**
for edits. Kept this optional local build for the final comparison.

Matching LLVM 22.1.2 tools merged the raw profiles. Builds used an explicit
`x86_64-unknown-linux-gnu` target, `-Cprofile-generate`, then
`-Cprofile-use` with `-Cllvm-args=-pgo-warn-missing-function`. The optimized
application build completed without missing-profile warnings.

## Layout and PGO throughput checkpoint

This checkpoint compares commit `e5031f9` with the new portable release, an
optional PGO release, and Alacritty 0.17.0 (`94e7c887`). Each workload uses
4,194,304 lines, five trials per binary, rotated execution order, and a
two-second warmup. Both terminal grids must remain 80 by 21 and return the
expected final cursor position. Byte counts must match within each mode.

Stages 10–15 and final launches use affinity `2,4,6,8` on the Intel Core
i7-12700. Stages 8–9 predate explicit CPU pinning.
Runs use native Wayland and Intel UHD Graphics 770. Fonts use 17 pt
`monospace`, resolved locally to FantasqueSansM Nerd Font Mono. Temporary
KDE and system inhibitors keep the display on and the machine awake during
measurement. Other user applications continue running, so these results
describe this machine and session. These conditions differ from stages
1–7; compare binaries within each recorded experiment.

Median seconds over 4,194,304 lines:

| Workload | Before | Portable | PGO | Alacritty |
| --- | ---: | ---: | ---: | ---: |
| Plain | 2.051 | 1.595 | 1.550 | 2.228 |
| SGR colors | 2.293 | 1.944 | 1.831 | 2.811 |
| Unicode | 2.128 | 1.866 | 1.837 | 2.174 |
| OSC 8 | 1.527 | 1.418 | 1.329 | 1.592 |
| Truecolor | 1.886 | 1.664 | 1.506 | 2.489 |
| Cursor edits | 7.626 | 1.411 | 1.305 | 2.509 |

Plain-output median peak RSS: before **148.9 MiB**, portable
**136.4 MiB**, PGO **133.8 MiB**, and Alacritty **113.8 MiB**.
Both new builds finished sooner than each control in all 30 paired
workload trials. Ratios divide the control median by the candidate
median; they describe the measured parser/PTY workloads.

The records include processing-period terminal CPU time from `/proc`,
startup-inclusive process-tree CPU time, peak RSS, geometry, and effective
CPU affinity. Binary hashes live in metadata; GPU logs live in the archive.
Cursor replies delimit parser/PTY throughput; frame presentation requires
separate measurement.

## 16 Measure software response latency

Each probe writes a marker plus CSI 6n and measures the cursor reply.
Two rotated trials collect 500 samples each per terminal and condition.
The primary interval starts after acquiring the producer lock. A separate
request-to-reply series includes that lock wait. Reported percentiles pool
the 1,000 samples per terminal/condition with nearest-rank p95/p99.

Conditions cover idle, a cumulative 64 MiB/s output target with catch-up
bursts, and unpaced saturation output. Recorded achieved rates allow the
paced trials to be checked against their target. Saturation rates depend
on each terminal's throughput.

Initial software response latency in milliseconds:

| Build | Idle median | Idle p99 | Paced p99 | Saturated p99 |
| --- | ---: | ---: | ---: | ---: |
| Before | 0.314 | 15.546 | 17.126 | 2.975 |
| Compact portable | 0.301 | 16.134 | 17.293 | 2.539 |
| Compact PGO | 0.300 | 15.838 | 17.479 | 2.375 |
| Alacritty | 0.216 | 0.774 | 4.934 | 3.047 |

The GUI thread spent 14–20 ms in GPU timeline-wait ioctls during a
diagnostic trace. A software-renderer control measured idle p99 at
3.311 ms. These observations motivated a presentation-policy trial.

These measurements cover software PTY processing and reply delivery.
Keyboard delivery, compositor scheduling, scanout, and pixel response fall
outside the timer. Physical key-to-photon latency needs separate input and
presentation instrumentation or external measurement.

## 17 Prefer Mailbox presentation when supported

The GPU renderer now selects Mailbox when the surface/adapter capabilities
advertise it, with AutoVsync as the fallback. Winit frame-callback pacing
continues through `pre_present_notify()`. The startup diagnostic records
the selected mode. The [wgpu PresentMode documentation][present-mode]
describes Mailbox as synchronized presentation with replacement of the
pending frame.

A pilot selected Mailbox on Intel Vulkan and measured idle p99 at
**1.179 ms**, compared with roughly 16 ms in the preceding GPU runs.
Rotated idle, paced, and saturation controls follow this pilot.

Paired confirmation, p99 milliseconds over 1,000 samples per entry:

| Build | Idle | Paced | Saturated |
| --- | ---: | ---: | ---: |
| AutoVsync | 15.847 | 17.669 | 2.274 |
| Mailbox | 1.124 | 0.909 | 2.299 |
| Alacritty | 0.683 | 5.020 | 3.055 |

### Final throughput after the presentation change

Repeated the same five-trial comparison after selecting Mailbox and
retraining PGO from fresh GPU/software profiles. Each run processes
4,194,304 lines. Median seconds:

| Workload | Before | Portable | PGO | Alacritty |
| --- | ---: | ---: | ---: | ---: |
| Plain | 1.490 | 1.255 | 1.242 | 1.941 |
| SGR colors | 1.816 | 1.549 | 1.457 | 2.495 |
| Unicode | 1.741 | 1.557 | 1.488 | 1.927 |
| OSC 8 | 1.513 | 1.452 | 1.345 | 1.584 |
| Truecolor | 1.934 | 1.624 | 1.515 | 2.486 |
| Cursor edits | 7.702 | 1.459 | 1.265 | 2.494 |

Plain-output median peak RSS, in the same order: **148.8 MiB**, **136.2 MiB**,
**133.8 MiB**, **114.2 MiB**.

Final software response latency in milliseconds:

| Build | Idle median | Idle p99 | Paced p99 | Saturated p99 |
| --- | ---: | ---: | ---: | ---: |
| Before | 0.296 | 15.506 | 16.499 | 3.010 |
| Mailbox portable | 0.227 | 1.005 | 0.924 | 2.642 |
| Mailbox PGO | 0.177 | 0.811 | 0.934 | 2.474 |
| Alacritty | 0.193 | 0.640 | 4.610 | 3.124 |

Two native-window screenshots show frame 2 and frame 5 with changing
colors, Unicode, emoji, and text attributes. This checks that the
lower software reply latency accompanies continuing visible updates.

PGO remains an optional local build. The normal release retains portable
compiler flags. Both final executables and profiles are archived.

[present-mode]: https://docs.rs/wgpu/30.0.1/wgpu/enum.PresentMode.html

## Follow-up validation and artifacts

The retained code passed 117 default tests, interactive fish, tmux and
Neovim checks, GPU/software pixel parity, formatting, and Clippy with
warnings denied. Fresh independent reviews covered the content layout,
erasure semantics, style conversions, presentation selection, and the
latency harness.

[Follow-up measurements](benchmarks/2026-10-16-follow-up.json) contain
stage summaries, final raw throughput runs, software latency summaries,
build hashes, and measurement metadata. The accompanying experiment
archive retains the full per-sample latency records and diagnostic logs.

The normal release build uses portable flags. The optional local PGO
executable lives at `target/pgo/cryptty`; its profile belongs to this source
and compiler snapshot. Source changes call for retraining. The experiment
archive retains training inputs, raw profiles, merged data, harnesses,
candidate patches, binaries, and individual measurements.

## One-command PGO build

From the repository root, install compiler-matched LLVM tools once:

```fish
rustup component add llvm-tools
```

Train, compile, and run:

```fish
./scripts/build-pgo
./target/pgo/cryptty
```

The script builds for the active Rust compiler's host target. It locates
`llvm-profdata` inside that compiler's sysroot, collects fresh profiles
with `-Cprofile-generate`, merges them, then builds with `-Cprofile-use`
and missing-function warnings enabled. Each completed invocation removes
its training checkpoint. A new unique merged-profile path forces a fresh
profile-use build on the following invocation.

Training opens an automatic-renderer window followed by a software-renderer
window. Automatic mode exercises GPU rendering when a compatible adapter
can initialize. The workload covers logs, SGR, Unicode, combining text,
OSC 8, truecolor, erasure, and paced styled-screen updates. Cursor replies
synchronize each phase. A completion file confirms that the child workload
finished before the script accepts the collected profile.

Run from an active desktop session. The workload prefers
`~/.venv/bin/python`; otherwise it uses `uv run --offline --script` with an
installed Python 3.11 or newer. It has no third-party Python dependencies.

The finished executable and merged profile live at `target/pgo/cryptty`
and `target/pgo/cryptty.profdata`. Cargo artifacts live in
`target/pgo-build/`. Interrupted or failed optimized builds retain their
checkpoint in `target/pgo/.training/`. Retrying with unchanged source,
scripts, and compiler reuses that profile and Cargo's build cache. Changed
inputs discard the checkpoint and retrain. Successful builds remove it. Output files stage on the same filesystem
and replace the executable through a rename, so a running terminal can
continue using its original executable.
The script serializes builds with `target/pgo/.build-lock`; after an
uncatchable termination such as SIGKILL, check for a running build before
removing a stale lock directory.

To train and install with Cargo:

```fish
./scripts/build-pgo --install
```

The script runs `cargo install --path . --locked --target <host> --force`
with the profile-use flags and shared build directory. Cargo's normal
installation settings, including `CARGO_INSTALL_ROOT`, apply. A plain
`cargo install --path .` continues to produce a normal release build.

The measurements above refer to the archived experimental builds and their
recorded hashes; a new invocation collects a new profile.

For the normal release, run `cargo build --release --locked`, then
`./target/release/cryptty`.

## Font range selection and cell fitting

The subsequent font changes add `font.codepoint_map` with multiple ranges
per family and later-entry precedence. Both rendering paths load styled
mapped faces. That version fitted fallback symbols to the primary cell and
used a full-block reference to preserve fractional geometry. These changes
follow the archived measurements above and have separate rendering checks.
The historical performance tables retain their original source snapshots.

Current rendering normalizes fallback font sizes per family against the
primary cell advance. Styles share the regular face's scale. Per-glyph bitmap
fitting and block transforms have been removed; baseline placement and font
mappings remain. These changes have no new performance measurements here.

Validation on Linux passed 125 default tests, Clippy with warnings denied,
and the explicit GPU/software pixel comparison across font sizes, range
mappings, styles, and window sizes. A separate fault-injection check covered
failed optimized builds, unchanged-source retries, Cargo installation,
successful-build retraining, and source-change invalidation. Raster previews
used 17 pt at display scale 1.25 with the default fallback, DejaVu Sans Mono,
and Liberation Mono. The macOS follow-up below records subsequent runtime
checks; CoreText's internal fallback cascade can differ from Linux's order.

The end-to-end PGO installation check used an isolated
`CARGO_INSTALL_ROOT`, exercised Intel UHD Graphics 770 through Vulkan
Mailbox plus software rendering, and produced identical installed and
local executables. The active terminal retained its original inode during
output replacement. Temporary checkpoints and the build lock were removed.

## macOS follow-up, 2026-10-16

The macOS comparison shows improved Unicode and OSC 8 throughput and lower
cursor-reply latency under load. Bulk plain text, truecolor, and cursor edits
regressed with GPU rendering. The cause requires further investigation.

[Raw measurements, build hashes, and harnesses][macos-bench]
preserve the runs, configurations, summaries, and validation outcomes.
Recorded commands retain historical temporary paths; their binaries have
been removed.

[macos-bench]: benchmarks/2026-10-16-macos.json

### Setup and measurement scope

Apple M4 Pro, 24 GiB RAM, macOS 26.6 (25G72), Rust 1.100.0-nightly
(`67eda617e`, LLVM 23.1.1). Both Cryptty versions used standard
`cargo build --release --locked` builds without PGO. The comparison uses
`9cec430` (before), `cab36c5` (current), and Alacritty 0.15.1 (`0c405d5`).
Each terminal used a 94 x 26 grid and 10,000 history rows through a direct
PTY. Current Cryptty selected Metal AutoVsync. The Linux measurements used
Vulkan Mailbox and different workloads and Alacritty versions.

Cursor replies delimit PTY/parser completion. Frame presentation and
physical input latency fall outside these measurements. Runs took place
in a normal desktop session with other applications running, without CPU
affinity or compositor instrumentation. Native visual appearance remains
outside this validation.

### Bulk throughput

Median seconds for 1,048,576 lines per workload, with five trials per
binary in rotated order and a warmup before each workload. Lower values
indicate faster completion. All three terminals used GPU rendering.

| Workload | Before | Current | Alacritty |
| --- | ---: | ---: | ---: |
| Plain text | 0.853 | 2.005 | 0.495 |
| Colors | 0.736 | 0.719 | 0.462 |
| Unicode | 0.803 | 0.271 | 0.485 |
| OSC 8 links | 0.822 | 0.573 | 0.650 |
| Truecolor | 0.799 | 2.557 | 0.555 |
| Cursor edits | 1.832 | 2.217 | 0.682 |

A software-renderer control used three trials per Cryptty binary with the
same workloads. Median seconds:

| Workload | Before software | Current software |
| --- | ---: | ---: |
| Plain text | 0.854 | 0.741 |
| Colors | 0.741 | 0.443 |
| Unicode | 0.799 | 0.309 |
| OSC 8 links | 0.844 | 0.469 |
| Truecolor | 0.818 | 1.596 |
| Cursor edits | 1.841 | 0.772 |

Software rendering improved five workloads relative to the prior software
build. Truecolor took about twice as long in the current software build.

### Cursor-reply latency

Two trials per binary pooled 1,000 measured samples per condition after
warmup. Each cell gives median / p99 milliseconds, timed from acquisition
of the producer lock through writing the query and receiving its reply.
The archive also includes request timings that include lock contention.

| Condition | Before | Current | Alacritty |
| --- | ---: | ---: | ---: |
| Idle | 14.150 / 15.298 | 0.104 / 14.607 | 0.119 / 1.484 |
| 16 MiB/s output | 6.673 / 15.009 | 0.151 / 0.808 | 0.072 / 2.167 |
| Saturated output | 0.928 / 1.134 | 0.037 / 0.203 | 1.160 / 2.375 |

Idle tail latency remained near one display interval. At 16 MiB/s,
current request p99 including producer-lock contention measured 2.513 ms,
compared with 15.033 ms before and 3.421 ms for Alacritty.

The saturated query workload achieved 208-215 MiB/s in current Cryptty,
compared with 66-72 MiB/s before and 113-114 MiB/s in Alacritty. This
query-interleaved stream differs from the bulk plain-text test. A separate
plain-text control reproduced the GPU slowdown with 256-, 1,024-, and
16,384-line write batches, including a scheduler yield after 1,024-line
writes: current medians were 1.920-2.052 s versus 0.872-0.882 s before.
The effect of periodic queries on throughput needs further investigation.

### macOS validation

The explicit GPU/software pixel comparison passed. The default test run
stopped at `render::font::tests::requested_family_and_weight_match_coretext`
in `src/render/native.rs:238`: the measured left bearing was 0, with -2
expected. Library results were 29 passed, 1 failed, and 1 ignored; later
test binaries were skipped after that failure. That result belongs to the
archived benchmark snapshot.

Subsequent font work removed per-glyph bitmap fitting, preserved native
baseline-relative placement, normalized sizes per family, and retained
hidden CoreText fallback descriptors. The current macOS suite passes the
family/weight and placement checks; the latest full run passed 154 regular
tests. Linux all-target compilation and the release build also pass.
These checks supersede the test failure above. Performance measurements
for the subsequent font and keyboard changes remain pending; the tables
above retain their original source versions.


## Generated geometry and cell metrics follow-up

Generated masks now cover U+2500–U+259F ahead of font mappings. Both
renderers share the cached masks. The extra top pixel has been removed
from cell height and baseline together; one bottom pixel remains for
rasterized descenders. Native Fantasque at 19 pt and 2× scale now uses a
41-pixel cell height, down from 42 pixels; its font line height measures
40 pixels. Menlo at size 19 requires the retained bottom pixel.

Geometry tests and the expanded Metal/software pixel comparison cover
all 160 generated characters. The full suite passes 159 regular tests.
These changes follow the archived throughput runs; their performance
impact remains unmeasured. See [milestones](roadmap.md) for visual acceptance.
