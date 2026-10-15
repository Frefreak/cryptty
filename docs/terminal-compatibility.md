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

Paste uses Cmd-V on macOS, Ctrl-Shift-V on Linux, or Shift-Insert.
Multiline paste is bracketed when requested by the application. Escape
characters that could close a bracketed paste are removed from its text.

## Text styles

Both GPU and software rendering support bold, italic, bold-italic, dim,
inverse, conceal, strikethrough, overline, and single/double/curly/dotted/
dashed underlines. Underlines can have their own color. Font styles use
the selected family's faces, with fallback when a face is unavailable.

SGR colors support the 16-color palette, 256 indexed colors, and RGB,
including semicolon and colon forms. Palette/default-color changes also
affect cells that were written before the change.

## OSC integration

| Commands | Behavior |
| --- | --- |
| 0, 2 | Window title |
| 4, 104 | Set/query/reset palette entries |
| 8 | Hyperlink metadata and modifier-click activation |
| 10, 11, 12 | Set/query foreground, background, and cursor colors |
| 110, 111, 112 | Restore configured default colors |
| 52 | Base64 text writes to the system clipboard (`c` or empty target) |

OSC accepts BEL and ST termination. Colors accept `rgb:r/g/b` and
hexadecimal forms, plus `black` and `white`. Hyperlinks open HTTP, HTTPS,
and mailto URLs through the platform opener. Clipboard reads through OSC
52 are ignored; pasting reads the clipboard on a user shortcut.

## Boundaries

This covers a practical subset of xterm behavior. Image protocols,
extended keyboard protocols, shell-integration markers, text blinking,
and native text selection are outside this milestone. Scrollback remains
the responsibility of tmux.

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
