# Inline images

Display PNG or JPEG files with the bundled command:

```fish
cryptty image picture.png
cryptty image first.png second.png
cryptty image --pane picture.png
```

Run this in the terminal where the images should appear. Output must be a
TTY. Images fit the current pane while keeping their aspect ratio. The
sender reduces large images before transmission and does not upscale small
images before upload. Placements round up to whole terminal cells.
When pixel dimensions
are unavailable, sizing uses an 8-by-16 pixel cell estimate.

## tmux and SSH

Use `--pane` inside tmux to open a right-side preview occupying 35% of the
window width. Press Enter in that pane to close it and restore the saved
layout. The default command continues to display in the current terminal.
`--pane` also works when the
calling tool captures stdout, provided `TMUX` and `TMUX_PANE` identify the
target session and pane. Paths are resolved before opening the preview.
Opening a preview selects its source window before uploading the image.

Before uploading, the sender queries terminal identity to count up to three
nested tmux layers, then confirms graphics support through that path.
Detection requires a secondary device attributes (DA2) response.
Each query waits at most two seconds for a response. Missing responses or
unsupported graphics produce an error before any image upload. Identity
queries avoid changing tmux pane titles while discovering passthrough layers.
Errors identify the failed query (DA2 identity or Kitty graphics) and its
passthrough depth: zero addresses the immediate terminal; each additional
level wraps the query through one detected tmux layer. Graphics errors also
include the preceding DA2 reply. Timeouts report the received input byte
count without printing unrelated terminal input. A timeout can result from
missing query support, blocked passthrough, or reply routing; the message
alone does not determine which cause applies.

When a local tmux session is available, the sender enables
`allow-passthrough on` for the current pane. The pane setting remains
enabled after the command exits; other panes and global settings are
unchanged. Failure to enable passthrough reports an error before upload.
The query briefly focuses its local sending pane when necessary, then
restores the prior pane unless focus has changed meanwhile. Over SSH, keep
the local SSH pane focused so tmux routes replies back to the sender.
The temporary preview pane uses `allow-passthrough all` so uploads can pass
through split redraws. This setting ends when the preview pane closes.
Image positions use ordinary text cells, so tmux can redraw them after window
switches and resize. After attaching from a new terminal, run the image
command again to populate that terminal's image cache. Up to three nested
tmux layers are covered by PTY tests. Each layer must allow passthrough.

Over SSH, run a compatible sender on the remote machine. Image bytes travel
through the terminal connection; Cryptty never reads remote file paths.
An SSH sender also probes through tmux on the local machine, even when
`TMUX` is unset remotely. Enable passthrough in the local SSH pane with
`tmux set -p allow-passthrough on` before connecting. The remote sender
cannot configure that local tmux server. Direct SSH commands need a PTY:
`ssh -t HOST cryptty image picture.png`.

## Protocol scope

Cryptty implements a static subset of the [Kitty graphics protocol][kgp]:

- Direct byte uploads of RGB, RGBA, or PNG, with chunking and zlib support.
- Support queries, Unicode placeholder placements, and coordinate inheritance.
- Ordinary cursor placements, including anonymous images and multiple
  placements of the same image. `C=1` preserves the cursor position.
- Source cropping, pixel offsets, aspect-preserving cell sizing, and signed
  z-order above text, beneath text, or beneath colored cell backgrounds.
- Placement tracking through scrolling, resize/reflow, and alternate screens.
- Deletion by image/placement ID, cell, row, column, layer, or image-ID range.

Ordinary placements work directly in Cryptty. Inside tmux, use the bundled
sender's Unicode placeholders so tmux can track positions when panes move.
Full-screen erasure removes visible ordinary placements; overwriting text
leaves them in place. The primary and alternate screens keep separate images.

Animation, relative placements, image-number allocation (`I`), file transfer,
and shared-memory transfer remain unsupported. Yazi backend detection and
acceptance are deferred.

Regression cases cover anonymous PNG uploads and behaviors
documented in [Kitty's upstream graphics tests][tests], including cursor
movement, replacement, scrolling clips, and deletion. These tests run against
Cryptty's terminal model; the Kitty test suite itself requires Kitty internals.

Decoded images are limited to 64 MiB each and 8192 pixels per dimension.
Storage holds up to 128 images or 128 MiB; older uploads are evicted when
needed, so their old placeholders may become blank. GPU texture limits
also apply. Software rendering uses the same placement and sampling rules.

## macOS command access

The app contains the sender in `Contents/MacOS/cryptty`. Exposing it on
`PATH` is a manual user choice; installation leaves shell configuration
and `PATH` untouched. For an app installed in `~/Applications`, the direct
invocation is:

```fish
~/Applications/Cryptty.app/Contents/MacOS/cryptty image picture.png
```

A separate Cargo installation is unnecessary when using the bundled binary.

[kgp]: https://sw.kovidgoyal.net/kitty/graphics-protocol/
[tests]: https://github.com/kovidgoyal/kitty/blob/master/kitty_tests/graphics.py
