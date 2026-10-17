# Terminal milestones and remaining work

Status as of October 17, 2026. Completed implementation and remaining
runtime acceptance are tracked separately. Historical measurements and
validation counts remain in [performance.md](performance.md).

## Completed milestones

- Terminal core, selection, paste, scrolling, and Ctrl-click links.
- GPU rendering and optimization stages 1–17, with recorded throughput,
  memory, and latency comparisons.
- Native macOS font rendering, font mappings, shared family sizing, and
  generated box drawing/block glyphs for U+2500–U+259F.
- Cursor geometry: adjusted cell height and a beam width of 15% of a cell.
- Kitty and Xterm mode-2 keyboard negotiation, configurable text bindings,
  and manual configuration reload with Ctrl+Shift+R.
- Output/resize coalescing: 2 ms quiet period with a 5 ms maximum wait.
  The user confirmed improved stability.
- macOS GPU frame pacing at the reported monitor refresh interval.
  Software rendering and Linux retain the existing coalescing deadlines.
- macOS Option-as-Alt handling and native application packaging. The user
  confirmed working Control shortcuts and Spotlight discovery.
- Build/install targets for the macOS app and Linux desktop integration,
  currently using a placeholder icon.

The latest recorded packaging checkpoint passed 166 regular tests, with
six ignored tests, Linux compilation, formatting, and the release build.
These checks cover their recorded source snapshot. Desktop observations
apply to the exercised cases and platforms.

## Desktop acceptance

Continue native keyboard checks on macOS and Linux: held-key repeats,
IME composition, focus changes with modifiers held, and suspend/resume.
Kitty and Xterm mode-2 PTY tests cover direct and tmux paths; physical input
acceptance remains case-specific. See [terminal compatibility][compat].

Linux lock bits track local Caps Lock and Num Lock presses. Initial state
and changes while unfocused remain implementation work. Xterm modes 1 and
3 remain unsupported.

Compare font mappings, bold/italic faces, fallback glyphs, combining text,
emoji, generated boxes, and cursor geometry at multiple display scales.
Linux runtime appearance and GPU/software comparisons need desktop checks.
Manual configuration reload also needs broader desktop acceptance.

## Performance follow-up

Use the [current performance record][performance] and its linked raw data
for completed measurements, including the October 17 Alacritty comparison
and macOS GPU pacing trials. Older regressions belong to archived source
snapshots; reproduce them on current code before attributing a cause.

Remaining measurements:

- Physical key-to-photon latency and resize behavior.
- Direct versus tmux behavior under matched workloads, separating terminal
  rendering from tmux processing and application output.
- Font-mapping startup, new-glyph, cache-churn, and memory costs. Keep font
  size, display scale, renderer, compiler, and PGO profile constant.

Retrain PGO when the relevant source or compiler snapshot changes.

## Application icon

Three concepts have been generated for selection: a crypt portal with a
cursor, a cryptid mascot, and a C monogram with a cursor. Final selection,
small-size refinement, and packaging remain pending.

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

[compat]: terminal-compatibility.md
[performance]: performance.md
[color-window]: https://github.com/alacritty/alacritty/blob/master/alacritty/src/display/window.rs
[metal-color]: https://developer.apple.com/documentation/quartzcore/cametallayer/colorspace
