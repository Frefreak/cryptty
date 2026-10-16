# Terminal compatibility

Cryptty advertises `TERM=xterm-256color` and `COLORTERM=truecolor`.
The implementation uses Alacritty's `vte` parser with bounded storage.
The targets for this milestone are fish, tmux, and Neovim. Direct Codex
keyboard negotiation has PTY coverage; desktop acceptance remains pending.

## Screen and input behavior

- Cursor movement, saved cursor position/attributes, autowrap, scrolling
  margins, origin mode, insertion/deletion, and erasure.
- Primary and alternate screens: DEC modes 47, 1047, 1048, and 1049.
  Both grids resize; alternate-screen use preserves the primary screen.
- Primary-screen text and retained history reflow across soft wraps on
  width changes. Hard line breaks, styles, hyperlinks, and wide/combining
  characters survive reflow. Up to 10,000 history rows support recovery
  when widening or growing taller. Alternate screens resize as fixed grids.
  The minimum grid width is two columns to accommodate wide characters.
- Programmable tab stops and DEC line-drawing character sets G0/G1.
- Application cursor keys and application keypad.
- Bracketed paste (2004), focus reporting (1004), mouse click/drag/motion
  reporting (1000/1002/1003), and SGR mouse encoding (1006).
- Block, underline, and beam cursors, including blinking variants.
- Synchronized updates (2026), with a 150 ms timeout to resume drawing
  when an application leaves the mode enabled.
- Primary/secondary device attributes, cursor-position reports, mode
  queries, terminal version, and character-grid size reports.
- DCS status queries for SGR, scrolling margins, and cursor style.
  Terminfo queries recognize terminal name, color count, and truecolor.

Paste uses Ctrl-Shift-V on both platforms, or Shift-Insert.
Dragging selects text and copies it to the system clipboard on release.
Double-click selects a whitespace-delimited run, preserving punctuation
within paths and URLs. Clicking whitespace selects its run. Triple-click
selects a logical line across soft wraps, including its final newline.
Dragging extends these selections by whole words or logical lines.
Successive clicks must land in the same cell within 400 ms of each other.
Shift bypasses application mouse capture. Ctrl-Shift-N opens a new window
with a fresh shell and the original launch directory and renderer options.
Multiline paste is bracketed when requested by the application. Escape
characters that could close a bracketed paste are removed from its text.

Output and resize redraws wait for 2 ms of quiet, with a 5 ms deadline from
the first pending update. Continued output cannot extend that deadline.
PTY parsing, replies, and input delivery continue immediately. Local UI
redraws can share the pending delay; display scheduling can add further
latency. Application-requested synchronized updates retain precedence.

## Keyboard reporting

Kitty keyboard negotiation supports active-flag queries (`CSI ? u`), push
(`CSI > flags u`), pop (`CSI < count u`), and set/add/remove operations
(`CSI = flags ; mode u`). Each screen has a separate stack of up to 32
saved modes. Full reset clears both stacks. Bare `CSI u` retains its
legacy cursor-restore meaning.

The five enhancement flags support disambiguation, repeat/release events,
alternate key codes, all-key reporting, and associated text. Encoding
follows Ghostty, including `CSI 13;2u` for Shift Enter in Codex's flags-5
mode. Configured text bindings take precedence over built-in shortcuts;
built-in shortcuts take precedence over protocol encoding. With flags
zero, Xterm mode 2 applies when requested; otherwise legacy encoding applies.

Xterm `modifyOtherKeys` mode 2 supports enable (`CSI > 4;2 m`), reset
(`CSI > 4 m`, `CSI > m`, or `CSI > 4;0 m`), disable (`CSI > 4 n`), and
query (`CSI ? 4 m`). It persists across screen switches and clears on full
reset. Modified keys use `CSI 27;modifier;codepoint ~`; application keypad
sequences retain priority. Kitty mode takes precedence when enabled.

For Shift+Enter to send legacy Alt+Enter, use the
[README text binding](../README.md#keyboard-bindings). The isolated tmux
test verifies this mapping with `extended-keys off`, including ordinary
Enter and Ctrl+B detach. It sends `ESC` plus Return directly, bypassing
keyboard negotiation. Physical desktop input in Codex remains to be checked.

For negotiated Shift Enter in Codex inside tmux 3.6a, without that binding,
use these tmux server settings:

```tmux
set -s extended-keys on
set -s extended-keys-format csi-u
```

When attaching from Cryptty, run `tmux -T extkeys attach`. This declares
the capability for that client. Codex requests mode 2 when tmux reports
CSI-u formatting. With Xterm formatting, the tested Codex leaves the pane
in legacy mode and Shift Enter submits. The isolated CSI-u PTY test kept
two draft markers in one multiline composer and restored normal mode on
exit. Physical desktop input remains an acceptance check.

Current boundaries:

- Xterm modes 1 and 3 remain unsupported.
- The parser normalizes omitted and explicit-zero pop counts to one.
- macOS Caps Lock uses AppKit state. Linux lock bits track local presses;
  initial lock state and changes outside focus require further work.
- Modifier tracking covers presses received by the window; keys already
  held when focus arrives need native desktop acceptance.

## Glyph geometry

The primary font determines terminal cell dimensions, with one bottom pixel
reserved for rasterized descenders. Block and beam cursors occupy the full cell height.

Box Drawing and Block Elements (U+2500–U+259F) use generated cell-aligned
masks before font lookup, including explicit range mappings. Both renderers
share these masks. Light/heavy/double lines, dashes, rounded corners,
diagonals, fractional blocks, and quadrants follow cell geometry. Shades
use uniform full-cell alpha 64, 128, and 192. Bold/italic retain the same
generated shape; foreground/background colors and decorations still apply.

Each other family uses
one font size derived from a fixed reference in its regular face, shared by
all styles. References prefer CJK and emoji at two cells, then Latin and
symbols at one cell; a family lacking these references keeps the configured
size. Hidden CoreText fallback faces retain their native descriptors.
Glyph bitmaps and baseline-relative placement remain unchanged after
rasterization. Font mappings handle mixed-width designs within a family.
Drawing remains clipped to the allocated cells and viewport.

## Text styles

Both GPU and software rendering support bold, italic, bold-italic, dim,
inverse, conceal, strikethrough, overline, and single/double/curly/dotted/
dashed underlines. Underlines can have their own color. Font styles use
the selected family's faces, with fallback when a face is unavailable.
SGR 5 and 6 blink text at slow and rapid rates; SGR 25 disables blinking.

SGR colors support the 16-color palette, 256 indexed colors, and RGB,
including semicolon and colon forms. Palette/default-color changes also
affect cells that were written before the change.

## OSC integration

| Commands | Behavior |
| --- | --- |
| 0, 2 | Window title |
| 4, 104 | Set/query/reset palette entries |
| 8 | Hyperlink metadata and modifier-click activation |
| 9 | Desktop notification text when the window lacks focus |
| 10, 11, 12 | Set/query foreground, background, and cursor colors |
| 110, 111, 112 | Restore configured default colors |
| 52 | Base64 text writes to the system clipboard (`c` or empty target) |

OSC accepts BEL and ST termination. Colors accept `rgb:r/g/b` and
hexadecimal forms, plus `black` and `white`. Ctrl-click opens HTTP/HTTPS
through the platform opener. Plain URLs follow soft wraps; explicit line
breaks end detection. OSC 8 supplies the full target independently of its
label. Hovering underlines links; Ctrl-Shift-click bypasses mouse capture.
Trailing echoed `^C` is excluded from plain-URL detection and double-click
selection of URL-containing text. OSC 8 targets remain unchanged.
Clipboard reads through OSC
52 are ignored; pasting reads the clipboard on a user shortcut.

BEL requests window attention while unfocused. OSC 9 additionally sends
its text through macOS `osascript` or Linux `notify-send` (which must be
installed). Desktop presentation follows OS notification settings;
macOS attributes AppleScript notifications to Script Editor. Attention
and message requests each have a one-second rate limit. OSC 9;4 progress
updates are logged as unsupported. Notification text is limited to 1,024
characters with control characters removed.

Unsupported OSC logs contain command IDs, byte counts, and occurrence
counts, excluding payloads. Logs live at
`$XDG_STATE_HOME/cryptty/unsupported-osc.log`, defaulting to
`~/.local/state/cryptty/unsupported-osc.log`. Logging samples occurrences
at powers of two, tracks up to 32 command IDs, and rotates at 128 KiB.

## Boundaries

This covers a practical subset of xterm behavior. Image protocols and shell-integration markers remain outside this
milestone. Scrollback navigation remains the responsibility of tmux;
the internal history buffer supports resize recovery. CSI 3 J clears it.

The parser accepts up to 32 CSI parameters/subparameters. Excessive CSI
parameter lists are ignored. OSC storage is capped at 64 KiB, and full
buffers are ignored rather than applied as truncated commands. Titles
are limited to 1,024 characters. OSC strings with 16 or more
semicolon-separated fields are ignored. Unknown sequences are ignored; supported
mode queries report unsupported modes as such.

## Verification

The October 16 local checkpoint passed 161 regular tests and Linux
all-target compilation. The isolated tmux keyboard test also passed.
Default tests skip interactive application and GPU checks; invoke those
explicitly as shown below. Archived performance runs retain their own
validation counts in [performance.md](performance.md).

```sh
cargo test --all-targets
cargo test --test fish -- --ignored
cargo test --test applications -- --ignored
cargo test gpu_pixels_match_software -- --ignored
cargo check --target x86_64-unknown-linux-gnu --all-targets
```

Application tests use isolated configuration and temporary resources.
The Neovim test edits and saves multiline Unicode text, resizes, and
returns to the shell screen. The tmux test creates split panes, resizes,
and returns to the shell screen. GPU tests compare actual pixels against
the software renderer across font sizes, clipping, styles, and cursors.
Linux compilation can be checked from macOS; Linux clipboard and GPU
runtime behavior require a Linux desktop.

## References

- [XTerm control sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.pdf)
- [Alacritty terminal state](https://github.com/alacritty/alacritty/blob/v0.15.1/alacritty_terminal/src/term/mod.rs)
- [VTE parser](https://github.com/alacritty/vte/tree/v0.15.0)
