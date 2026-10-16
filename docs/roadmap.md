# Planned terminal work

The following rendering and keyboard work remains pending as of
October 16, 2026. Each item records the observed behavior and the checks
needed before treating a change as complete.

Completed optimization work appears in [performance.md](performance.md):
stages 1–17, compact cell storage, contiguous erasure, GPU presentation,
portable and PGO comparisons, throughput, memory, latency, and validation.
The document also covers the resumable PGO build and Cargo installation.
[Raw follow-up measurements](benchmarks/2026-10-16-follow-up.json) retain
trial data, build hashes, and measurement metadata. Those measurements
belong to their recorded source snapshots; font changes followed later.

## Generate box drawing and block glyphs

Status: planned implementation.

Current rendering obtains box drawing and block elements from fonts.
Range mappings and cell fitting improve font selection and geometry, while
shading retains the selected font's texture and margins. A probe using
Liberation Mono at 17 pt and display scale 1.25 produced a 14 by 32 pixel
full block and a 13 by 31 pixel light shade in a 14 by 32 pixel cell.
The user also observed the remaining shaded-bar texture and spacing.

Generate glyphs from cell geometry for Box Drawing, U+2500–U+257F, and
Block Elements, U+2580–U+259F. Cover connected lines, fractional fills,
quadrants, and full-cell shading. Alacritty and Ghostty provide useful
reference implementations; their shade glyphs use uniform coverage near
25%, 50%, and 75% for light, medium, and dark shades.

Before implementation, settle how explicit `font.codepoint_map` entries
interact with generated glyphs and any enable/disable setting. The current
user configuration explicitly maps block elements to Liberation Mono;
account for that mapping when adopting generated rendering. Retain font
selection for ordinary text and other symbols.

Acceptance checks:

- Adjacent full blocks and box borders meet at cell boundaries.
- Half, eighth, and quadrant blocks retain their defined proportions,
  including odd cell dimensions and fractional display scales.
- Shades cover the complete cell with consistent density and no seams.
- Software and GPU rendering agree across colors, text attributes,
  resizing, clipping, and supported display scales.
- Compare the same status bar and terminal borders with Alacritty and
  Ghostty at matching grid dimensions; verify override precedence.

References:

- [Unicode 17 section 22.8.1][u]
  describes continuous box lines and solid adjacent block patterns.
- [Alacritty generated glyphs][a]
- [Ghostty block sprites][g]

## Investigate cursor height and cell metrics

Status: reported visual issue; cause requires investigation.

The user reports an excessively tall cursor. Current `cursor_rect` in
`src/render/decoration.rs` gives block and beam cursors the full cell height.
Primary font metrics and a two-pixel addition determine cell height.
Their contribution to the reported appearance remains a hypothesis.
The shaded glyph's margins have separate measured evidence; establish any
shared cause before linking the two issues.

Reproduce with the configured Fantasque primary font at 17 pt. Record the
font metrics, baseline, cell rectangle, and cursor rectangle at each scale.
Compare block, beam, and underline cursors against Alacritty/Ghostty with
matching settings. Choose any metric or cursor adjustment from those
comparisons, preserving grid alignment, wide-character coverage, IME
placement, and GPU/software agreement.

## Negotiate keyboard reporting and distinguish Shift Enter

Status: deferred keyboard work.

Current input encoding sends carriage return for both Enter and Shift Enter.
The user confirmed Ctrl J as a working newline workaround in Codex.
The planned work includes application-driven keyboard protocol negotiation
and configurable bindings, with Ghostty as a design reference.

Follow the [Kitty keyboard protocol][k] for capability detection and mode
management. `CSI` below denotes the two bytes ESC followed by `[`.

- Answer `CSI ? u` with `CSI ? flags u` for the active enhancement flags.
  Test detection alongside primary device attributes, including replies
  split across PTY reads. Report flags consistent with implemented behavior.
- Handle `CSI > flags u` pushes, `CSI < number u` pops, and
  `CSI = flags ; mode u` updates. Bound the stack and maintain separate
  main-screen and alternate-screen stacks as specified by the protocol.
- Encode modifier-sensitive keys according to the active flags. For
  example, flag value 5 combines disambiguation and alternate-key reporting;
  verify Shift Enter as `CSI 13 ; 2 u` in the applicable enhanced mode.
- Preserve legacy input behavior outside enhanced modes. Verify mode
  restoration after application exit, suspend/resume, and screen switches.
- Validate the flags actually requested by the installed Codex build,
  directly and through tmux. Earlier research used a separate Codex source
  checkout, which probes with `CSI ? u` and pushes enhancement flags;
  terminal detection and tmux settings can affect its requested modes.

Define configurable modifier-sensitive bindings and raw sequence bindings
alongside protocol handling. Settle binding syntax, supported actions,
and precedence between explicit bindings and negotiated encoding before
implementation. An explicit Shift Enter binding and application-negotiated
reporting need separate tests.

Acceptance checks should capture exact PTY bytes, exercise mode stack
nesting and reset behavior, and run the installed Codex build both directly
and inside tmux. Confirm Enter submits and Shift Enter inserts a newline
in the intended Codex mode. Exercise fish and Neovim to verify ordinary
Enter, existing shortcuts, and restored shell input behavior.

## Validate font mappings on macOS

Status: Linux validation complete; macOS runtime checks pending.

Exercise primary and mapped families, styled faces, unavailable families,
missing glyphs, symbols, blocks, combining text, and color emoji. Check
CoreText's internal fallback cascade against the intended mapping order.
Compare cell fitting, baseline placement, and GPU/software output on a real
macOS desktop before claiming cross-platform visual parity.

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
