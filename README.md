A vibe-coded personal terminal that suits my needs.

Build and run a normal release from the repository root:

```fish
cargo build --release --locked
./target/release/cryptty
```

On macOS, build or install an application searchable as **Cryptty**:

```fish
make macos-app       # Build target/macos/Cryptty.app
make install-macos   # Install ~/Applications/Cryptty.app
```

These commands require Rust, uv, and the Xcode Command Line Tools.
The app contains a native launcher and a placeholder terminal icon, with
an ad-hoc signature for local use. It opens the inherited `$SHELL` as a
login shell in your home directory. Repeat the install command to update.
For a custom destination, use `./scripts/build-macos-app --destination PATH`;
add `--install` to register that location with macOS.
The app runs independently of this checkout and of uv.

On Linux, install the binary, desktop entry, and placeholder icon:

```fish
make install-linux
```

The default prefix is `~/.local`; ensure `~/.local/bin` is on your desktop
session's `PATH`. Override with `make install-linux PREFIX=/your/prefix`.
The desktop entry lives in `assets/cryptty.desktop` for distribution packaging.
Run `make help` for the available targets.

For profile-guided optimization (PGO), train and build with one command:

```fish
./scripts/build-pgo
./target/pgo/cryptty
```

Fresh training collects data in two short-lived windows:
automatic rendering, then forced software rendering. Automatic mode uses
a compatible GPU when available. Run this from an active desktop session.
Install matching LLVM tools first with `rustup component add llvm-tools`.
The workload uses `~/.venv/bin/python` when available, or `uv run --offline
--script` with an installed Python 3.11 or newer. Its dependencies use the
Python standard library.

The script saves the executable and merged profile under `target/pgo/`,
with build artifacts under `target/pgo-build/`. All remain local, ignored
artifacts. Interrupted builds retain a training checkpoint; retrying with
unchanged source and compiler resumes the optimized build. Successful
builds remove the checkpoint. To train and install through Cargo, run:

```fish
./scripts/build-pgo --install
```

This invokes `cargo install --path .` with the trained profile and the same
build cache. Cargo uses its normal installation directory.
See [the PGO workflow](docs/performance.md#one-command-pgo-build).

Rendering uses wgpu with Metal on macOS and Vulkan on Linux. Automatic
selection prefers an integrated GPU, then discrete and virtual GPUs.
If initialization or rendering fails, automatic mode switches to the
software renderer. CPU-only graphics adapters are skipped in automatic
selection. Software mode skips GPU initialization, for VMs and drivers
without suitable GPU support.

```sh
cryptty                         # Automatic GPU selection with fallback
cryptty --renderer software     # Force CPU rendering
cryptty --renderer gpu          # Require a compatible GPU
cryptty --list-gpus             # List adapter indices, names, and types
cryptty --gpu 0 -- fish          # Require the listed adapter
```

Explicit GPU requests report errors instead of falling back. Adapter
indices refer to the current enumeration; check the list again after
hardware or driver changes. Selection uses one adapter per process.
Font rasterization remains on the CPU; the GPU caches glyph images and
draws text, backgrounds, underlines, preedit, and the cursor.

The offscreen rendering comparison requires a working GPU:

```sh
cargo test gpu_pixels_match_software -- --ignored
```

Terminal compatibility covers alternate screens, bracketed paste,
focus/mouse reporting, richer text styles, and common OSC commands.
See [supported sequences and checks](docs/terminal-compatibility.md).
Output and resize redraws use a 2 ms quiet period, capped at 5 ms from the
first pending update, to combine closely spaced changes. PTY processing
and input delivery remain immediate; visible updates can share this delay.
On macOS, pending GPU redraws also wait for the next refresh interval to
keep the event loop available between frames.
Left Option acts as Alt on macOS, including Alt+h/j/k/l for configured
tmux/Vim navigation. Other Option keys use macOS character mapping;
ordinary Alt-modified key events retain the terminal's Escape prefix.
Set `window.option_as_alt` to `none`, `only_left` (default), `only_right`,
or `both` to change this; Ctrl+Shift+R applies it to the current window.
Linux ignores this setting.
Beam cursors scale to 15% of one cell's width, with a one-pixel minimum.
Drag to select and copy to the system clipboard. Paste with Ctrl-Shift-V,
Shift-Insert, or middle-click on both platforms. During application mouse
capture, use Shift-middle-click to paste from the system clipboard.
Double-click selects a whitespace-delimited
word; triple-click selects a logical line, including soft-wrapped rows.
Dragging after either click extends the selection by words or lines.
Selection survives unrelated output and follows scrolling while its text
remains visible. Overwriting the selected text, switching screens, resizing,
starting a new selection, or pasting clears it. Copying retains plain text.
Ctrl-Shift-N opens a fresh shell in
a new window, using the original launch directory and renderer options.
Ctrl-click opens HTTP/HTTPS links, including OSC 8 targets and plain URLs
across terminal soft wraps. Hovering underlines the detected link.
Hold Shift to select or Ctrl-Shift-click links during mouse capture.
Trailing echoed `^C` is excluded from plain URLs and their word selection.
Applications can copy text through OSC 52 to the system clipboard.

Resizing reflows primary-screen text and retains up to 10,000 history rows
for resize recovery. Alternate-screen applications retain a fixed grid and
receive the new size to redraw. Scrollback navigation remains with tmux.

Configuration lives in `$XDG_CONFIG_HOME/cryptty/cryptty.yaml`, or
`~/.config/cryptty/cryptty.yaml` when `XDG_CONFIG_HOME` is unset.
Cryptty creates a missing file with plain commented YAML defaults.
Omitted settings follow the current built-in defaults. To override a
setting, add normal nested YAML. If its parent key already exists, place
the setting under that key:

```yaml
font:
  family: FantasqueSansM Nerd Font Mono
  size: 19
```

Font size uses points. On startup and reload, Cryptty appends comments for
missing fields. A matching comment under the same keys prevents a
repeat notice. Existing values and comments stay intact. No block markers
or version metadata are required.

Press **Ctrl+Shift+R** to reload the configuration in the current window.
Font settings, padding, colors, and keyboard bindings apply immediately;
the shell and terminal contents stay in place. Reload reapplies the color
palette and OSC color reset defaults. A failed reload retains the active
settings, adds `config reload failed` to the window title, and reports the
error to stderr. A successful retry clears that title message. Explicit
text bindings retain precedence over built-in shortcuts.

On macOS, fonts use CoreText through crossfont and follow the system's
font-smoothing setting. On Linux, fonts use Swash; `font.hinting` applies
to that renderer.

Choose fonts for one or more separate Unicode ranges with `codepoint_map`:

```yaml
font:
  codepoint_map:
    - ranges: ["U+23CE", "U+25B2", "U+25BC", "U+2580-U+259F"]
      family: DejaVu Sans Mono
```

The built-in defaults use FantasqueSansM Nerd Font Mono at 19 pt, with
LXGW WenKai Mono Screen mapped to CJK ideographs, punctuation, and fullwidth
forms. Install both families to obtain this appearance. The ranges are
`U+2E80-U+9FFF`, `U+F900-U+FAFF`, `U+FE10-U+FE1F`, `U+FE30-U+FE4F`,
`U+FF00-U+FFEF`, and `U+20000-U+3FFFF`. An explicit `codepoint_map` replaces
the default list; use `codepoint_map: []` to disable it.

On macOS, the defaults also map `⏎ ▲ ▼` (`U+23CE`, `U+25B2`, `U+25BC`)
and box drawing/block elements (`U+2500-U+259F`) to the system Menlo font.
The renderer generates U+2500–U+259F directly from cell geometry, taking
priority over font mappings. Existing mappings for these ranges may remain
in the config. Shades fill the whole cell with uniform 25%, 50%, or 75%
coverage; ordinary symbols such as `⏎ ▲ ▼` still use font mappings.

Add multiple entries for different families; later entries take priority
where ranges overlap. Ranges accept Unicode scalar values in `U+HEX` or
`U+HEX-U+HEX` form. Bold and italic select matching faces when available.
An unavailable family warns and falls back to the primary family. A missing
glyph follows the normal fallback path.

The primary family and size define the grid, with one bottom pixel
reserved for rasterized descenders. Block and beam cursors cover the cell height. Other families receive a shared
font size measured from their regular face: CJK and emoji references target
two cells, Latin and symbol references target one. All styles share that
size. Families without a supported reference retain the configured size.
Glyphs retain the rasterizer's baseline-relative placement, with no bitmap
resizing or centering. Drawing is clipped to the allocated cells. Use font
mappings for mixed-width designs, such as one-cell triangles in a CJK font.

## Keyboard bindings

Bindings send text directly to the application, ahead of built-in shortcuts
and negotiated keyboard encoding. For Shift+Enter to send legacy Alt+Enter:

```yaml
keyboard:
  bindings:
    - key: Enter
      modifiers: [Shift]
      text: "\u001b\r"
```

Keys accept names such as `Enter`, `Tab`, `Escape`, `ArrowUp`, and `F1`, or
one character from the unmodified keyboard layout, such as `v`. Modifiers
accept `Shift`, `Control`, `Alt`, and `Super` (Command on macOS). Matching
requires exactly the listed modifiers; an omitted list means none. Later
bindings take priority. Empty text consumes the key without sending bytes.
YAML double-quoted strings support escapes such as `\u001b`, `\r`, and `\n`.
Bindings repeat while held and retain their initial match until release.
Text composition takes priority. Press Ctrl+Shift+R after changing bindings.
The example sends `ESC` plus Return and works through tmux with
`extended-keys off`; the isolated PTY test verifies the emitted bytes.
Bindings currently send text. Action bindings and automatic file watching
remain outside this implementation.

See [performance experiments and measurements](docs/performance.md).
See [milestone status and remaining work](docs/roadmap.md).
