A vibe-coded personal terminal that suits my needs.

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
on both platforms, or Shift-Insert. Ctrl-Shift-N opens a fresh shell in
a new window, using the original launch directory and renderer options.
Ctrl-click opens HTTP/HTTPS links, including OSC 8 targets and plain URLs
across terminal soft wraps. Hovering underlines the detected link.
Hold Shift to select or Ctrl-Shift-click links during mouse capture.
Applications can copy text through OSC 52 to the system clipboard.

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
