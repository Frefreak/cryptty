# Follow-up notes

Reviewed October 18, 2026. The completed milestone checklist has been
retired. These notes retain deferred ideas and outstanding checks without
assigning them to a release.

## Compatibility and desktop checks

- Linux Caps Lock and Num Lock bits track local presses. Initial state and
  changes while unfocused still need synchronization. Xterm
  `modifyOtherKeys` modes 1 and 3 remain unsupported. See
  [terminal compatibility](terminal-compatibility.md) for protocol scope.
- Further native input checks include IME composition, held-key repeats,
  focus changes with modifiers held, and suspend/resume.
- Further visual checks include font mappings, fallback glyphs, combining
  text, emoji, and cursor geometry across display scales and renderers.
- Actual SSH image acceptance, nested multiplexers, and Yazi integration
  remain deferred. See [image usage and protocol scope](images.md).

## Measurements and optional ideas

See [performance experiments and measurements](performance.md) for recorded
results and source snapshots. Possible follow-up measurements include
physical key-to-photon latency, resize behavior, matched direct/tmux
workloads, and font-mapping startup, cache-churn, and memory costs.

Explicit Cryptty environment markers and propagation through tmux, SSH,
and agent subprocesses remain deferred. A user-local terminfo entry could
advertise terminal capabilities; tmux retains its own terminal type.
Captured tool output and terminfo cannot reliably identify an agent's
frontend.

## Color appearance across platforms

Status: macOS visual difference reported on October 17; investigation
deferred, with no rendering change made for this report.

The user supplied matching Codex status-line crops: Cryptty in the first
image, Alacritty in the second. Cryptty's cyan, pink, and yellow appeared
more saturated. The crops also showed different text sizes. The local
primary, normal, and bright palette RGB values matched between both apps.

Color-space handling remains a candidate explanation. Alacritty's upstream
macOS window code explicitly selects sRGB. The inspected wgpu-hal 30.0.1
Metal path sets the layer color space to nil for its sRGB surface choice;
Apple documents nil as disabling color matching. A controlled comparison
with explicit sRGB tagging is needed to establish causality.

The user reports similar appearance between both terminals on Linux.
Whether Linux can reproduce the preferred macOS vividness remains open;
no Linux display measurements or color-profile comparison were performed.
Future checks should use identical content, font size, display, and known
color profiles, including solid color patches to separate color handling
from text rasterization. Confirm the desired appearance before changing
color policy on either platform.

References: [Alacritty window code][color-window] and
[Apple CAMetalLayer color-space documentation][metal-color].

[color-window]: https://github.com/alacritty/alacritty/blob/master/alacritty/src/display/window.rs
[metal-color]: https://developer.apple.com/documentation/quartzcore/cametallayer/colorspace
