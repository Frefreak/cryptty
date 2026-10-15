# Terminal compatibility

Cryptty advertises `TERM=xterm-256color` and `COLORTERM=truecolor`.
The implementation uses Alacritty's `vte` parser with bounded storage.
The targets for this milestone are fish, tmux, and Neovim. Codex acceptance
is performed by the user.

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

This covers a practical subset of xterm behavior. Image protocols,
extended keyboard protocols and shell-integration markers remain outside
this milestone. Scrollback navigation remains the responsibility of tmux;
the internal history buffer supports resize recovery. CSI 3 J clears it.

The parser accepts up to 32 CSI parameters/subparameters. Excessive CSI
parameter lists are ignored. OSC storage is capped at 64 KiB, and full
buffers are ignored rather than applied as truncated commands. Titles
are limited to 1,024 characters. OSC strings with 16 or more
semicolon-separated fields are ignored. Unknown sequences are ignored; supported
mode queries report unsupported modes as such.

## Verification

```sh
cargo test
cargo test --test fish -- --ignored
cargo test --test applications -- --ignored
cargo test gpu_pixels_match_software -- --ignored
cargo check --target x86_64-unknown-linux-gnu
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
