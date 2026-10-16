# Terminal milestones and remaining work

Status as of October 16, 2026. Implementation, automated validation, and
desktop acceptance have separate status below.

## Milestone status

| Area | Status | Remaining work |
| --- | --- | --- |
| Terminal core and interaction | Implemented, tested | Desktop acceptance |
| GPU and optimization stages 1–17 | Implemented, measured | macOS regressions |
| Font mapping and normalization | Implemented, tested | Visual checks, timing |
| Kitty and Xterm mode 2 | Implemented, PTY tested | Desktop input lifecycle |
| Configurable text bindings | Implemented, tmux tested | Codex physical input |
| Generated boxes and blocks | Implemented, pixel tested | Desktop comparison |
| Cursor height | Extra top pixel removed | Desktop appearance check |

The latest local checkpoint passed 159 regular tests, Linux compilation of
all targets, formatting, and a release build. The isolated tmux keyboard
test also passed, including the configured Shift+Enter binding with
`extended-keys off`. These checks exercise configuration and PTY bytes;
physical key delivery and the native desktop remain separate checks.

## Completed performance work

Completed optimization work appears in [performance.md](performance.md):
stages 1–17, compact cell storage, contiguous erasure, GPU presentation,
portable and PGO comparisons, throughput, memory, latency, and validation.
The document also covers the resumable PGO build and Cargo installation.
[Raw follow-up measurements](benchmarks/2026-10-16-follow-up.json) retain
trial data, build hashes, and measurement metadata. Those measurements
belong to their recorded source snapshots; font changes followed later.

## Generated box drawing and block glyphs

Status: implemented for U+2500–U+259F, with shared CPU/GPU glyph masks.

Generated geometry takes priority over `font.codepoint_map` for these
ranges. Other characters retain normal font selection. The renderer covers
light, heavy, mixed, double, dashed, rounded, and diagonal lines; fractional
blocks and quadrants; and full-cell shades at alpha 64, 128, and 192.
Bold and italic retain the generated geometry. Cell dimensions determine
line thickness and fractional coverage, including odd pixel dimensions.

Checks cover all 160 characters over small, odd, and even cell sizes,
block area and complementary fills, uniform shades, connected edges,
double-line channels, and mapping/style precedence. The expanded Metal
comparison passes against software rendering across font sizes, clipping,
colors, decorations, cursors, and fractional sizing.

Desktop comparison against the same status bars and borders in Alacritty
and Ghostty remains useful. The earlier Liberation Mono measurements in
performance work describe font-rendered blocks before this change.

## Cursor height and cell metrics

Status: removed the extra top pixel; native font measurements and rendering
checks pass. Final desktop appearance remains to be checked.

Block and beam cursors continue to cover the cell. The rasterizers added
two pixels to cell height and one pixel to the baseline. The extra top
pixel has been removed from cell height and baseline together. One bottom
pixel remains to preserve rasterized descenders beyond rounded font metrics;
Menlo at size 19 requires this space.

Native macOS measurements for Fantasque at 19 pt:

| Display scale | Font line height | Previous cell height | Current height |
| --- | ---: | ---: | ---: |
| 1 | 20 px | 22 px | 21 px |
| 1.25 | 25 px | 27 px | 26 px |
| 2 | 40 px | 42 px | 41 px |

The 17 pt probe also recorded a one-pixel reduction at each scale.
Descender and underscore tests cover 16, 19, 20, 23.75, 32, and 38 raster
sizes with the default and fallback fonts. Wide block cursors, preedit, and GPU/software comparisons pass.
This establishes the padding contribution; the user's final visual
assessment remains a desktop check. Linux runtime validation remains open.

## Negotiate keyboard reporting and distinguish Shift Enter

Status: Kitty and Xterm mode-2 negotiation implemented. Codex PTY acceptance
passed directly and through tmux with extended keys and CSI-u formatting.
Configurable text bindings support exact modifiers and raw escape sequences.

The implementation follows Ghostty's Kitty reporting behavior: active flags
can be queried, pushed, popped, or updated, with separate bounded stacks
for the main and alternate screens. Flags 1, 2, 4, 8, and 16 cover key
disambiguation, events, alternate keys, all keys, and associated text.
Legacy encoding remains active when flags are zero.

The installed Codex 0.163.0 requested flags 5. A PTY probe using Cryptty's
parser and encoder produced `CSI 13;2u` for Shift Enter; two unsent draft
markers appeared on separate lines. Exit restored flags to zero. Encoder
checks cover ordinary Enter, nested modes, screen switches, resets,
repeat/release events, composed text, and keypad keys. Existing fish,
Neovim, and tmux shell integration checks passed.

The tmux 3.6a test enables Xterm mode 2 on the outer terminal. Exact-byte
tests cover both tmux output formats, ordinary Enter, Ctrl+B detach, and
mode restoration. For negotiated Shift Enter without a text binding, the tested Codex uses
`extended-keys on` and
`extended-keys-format csi-u`; attach from Cryptty with `tmux -T extkeys attach`.
The verified CSI-u probe retained two adjacent lines in one unsent draft.
Physical desktop input and suspend/resume remain acceptance work.

Keyboard bindings send configured text before built-in shortcuts and
protocol encoding. A Shift+Enter binding sending `ESC` plus Return works
through legacy tmux input with `extended-keys off`; the isolated test
checks exact bytes, ordinary Enter, and Ctrl+B detach. Bindings support
exact modifier matching, later-entry precedence, and captured repeats
until release. Active text composition suppresses bound output.
Configuration changes require a restart. See the README for syntax and
[terminal compatibility](terminal-compatibility.md) for
current keyboard boundaries and the [Kitty protocol][k] for the wire format.

## Complete desktop keyboard acceptance

Status: PTY and unit coverage complete for the recorded cases; native
input lifecycle needs acceptance on macOS and Linux.

Exercise Shift+Enter in Codex directly and through tmux, held-key repeats,
IME composition, focus changes with modifiers held, and suspend/resume.
Linux lock bits currently track local Caps Lock and Num Lock presses;
initial state and changes while unfocused require implementation work.
Xterm modes 1 and 3 remain unsupported. Text bindings currently send bytes;
action bindings and live configuration reload fall outside their scope.

## Validate font mappings on macOS

Status: automated macOS font checks pass; desktop visual acceptance pending.

The current suite covers requested family and weight, native glyph
placement, missing families, CJK and color emoji, hidden CoreText fallback
faces, and shared family sizing across styles. Swash family-size checks
also run on macOS; Linux all-target compilation passes.

Compare primary and mapped families, bold/italic faces, missing glyphs,
symbols, blocks, combining text, and emoji on both desktops. Inspect
baselines, clipping, fallback order, and GPU/software appearance at
multiple display scales. Glyphs retain native placement after family
normalization; the earlier per-glyph bitmap fitting has been removed.

## Investigate macOS performance regressions

Status: measured on archived source snapshots; cause remains open.

The macOS GPU comparison recorded slower bulk plain text, truecolor, and
cursor edits. The software control also recorded slower truecolor. Idle
cursor-reply p99 remained near one display interval. Periodic queries
changed observed throughput and require investigation. See the
[macOS measurements](performance.md#macos-follow-up-2026-10-16) for exact
versions, controls, and scope.

Reproduce on the current source before attributing these results to the
subsequent font or keyboard changes. Separate parser, glyph/cache, and
presentation costs. Physical key-to-photon latency needs independent
measurement. Retrain PGO for any new source/compiler snapshot.

## Measure font mapping overhead

Status: estimate from cache structure; measured comparison pending.

Range selection and fitting happen when a character/style enters the glyph
cache. Cached redraws reuse glyph bitmaps. Measure startup, new-glyph-heavy
screens, cached redraws, cache churn, and memory with mappings enabled and
disabled. Hold source, compiler, font size, scale, renderer, and PGO profile
constant. Record the resulting glyph/font differences alongside timings.
The existing throughput tables cover the earlier optimization snapshots.

[u]: https://www.unicode.org/versions/Unicode17.0.0/core-spec/chapter-22/
[a]: https://github.com/alacritty/alacritty/blob/master/alacritty/src/renderer/text/builtin_font.rs
[g]: https://github.com/ghostty-org/ghostty/blob/main/src/font/sprite/draw/block.zig
[k]: https://sw.kovidgoyal.net/kitty/keyboard-protocol/
