A vibe-coded personal terminal that suits my needs.

Build and run a normal release from the repository root:

```fish
cargo build --release --locked
./target/release/cryptty
```

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
Drag to select and copy to the system clipboard. Paste with Ctrl-Shift-V
on both platforms, or Shift-Insert. Double-click selects a whitespace-delimited
word; triple-click selects a logical line, including soft-wrapped rows.
Dragging after either click extends the selection by words or lines.
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
  family: Alacritty Fantasque WenKai Mono
  size: 19
```

Font size uses points. On startup, Cryptty appends default comments for
missing fields. A matching latest comment under the same keys prevents a
repeat notice. Existing values and comments stay intact. No block markers
or version metadata are required.

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

Add multiple entries for different families; later entries take priority
where ranges overlap. Ranges accept Unicode scalar values in `U+HEX` or
`U+HEX-U+HEX` form. Bold and italic select matching faces when available.
An unavailable family warns and falls back to the primary family. A missing
glyph follows the normal fallback path.

Mapped and fallback glyphs fit the primary font's cell dimensions without
changing Unicode column widths. Block elements share a full-block reference
so half blocks retain their proportions. Shading and outline details still
follow the selected font. The primary family and size define the grid.

See [performance experiments and measurements](docs/performance.md).
