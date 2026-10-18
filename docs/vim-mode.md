# Vim mode

Ctrl+Shift+J (the default) enters Vim mode and starts a Leap search. The
text stays frozen while the application continues running. A magenta frame
surrounds the view, and the hollow cursor keeps its character readable.
The window title shows the current mode, pending counts, and search input
stages.

Leap searches visible text in all supported panes of the current tmux
window. After landing, navigation stays within that pane. With a selection
active, further Leaps stay in its pane and extend from the existing anchor.
Outside tmux, the visible terminal grid supplies the text. This version
covers visible text; scrollback navigation can be added later.

## Leap

- Type two characters to find their pair. Labels appear for ambiguous targets.
- Type a displayed label to land there. Space advances the label group;
  Backspace returns to the previous group.
- The highest-ranked target is selected automatically when there are at
  most 18 matches, including matches spread across panes. The remaining
  targets use safe labels that avoid common navigation keys.
  A movement key continues from the automatic target.
- Enter after one character chooses the preferred match. Enter immediately
  after `s` repeats the last search, including one-character shortcuts.
- During an automatic jump, Enter and Backspace traverse its matches.
  The selected match keeps its original character beneath the hollow cursor,
  without a Leap highlight. Its label transfers to the vacated match so
  every other target remains selectable by letter.
- Lowercase queries ignore case. An uppercase character makes the whole
  pair case-sensitive. Spaces match line endings; brackets and quotes use
  Leap-style equivalence groups.
- Esc cancels the search and restores its starting cursor.

Target ranking favors the current pane, the current row, then forward
matches on that row. Distance uses narrower horizontal weighting. Label
previews, safe labels, and grouping follow the Leap.nvim interaction model.
Terminal cells provide coordinates, including wide and combining characters.

## Navigation and selection

| Key | Action |
| --- | --- |
| `h`, `j`, `k`, `l` | Move left, down, up, right |
| `w`, `b`, `e` | Next word, previous word, end of word |
| `W`, `B`, `E` | Equivalent motions across whitespace-separated words |
| `0` | Start of logical line |
| `^`, `H` | First nonblank character of logical line |
| `$`, `L` | End of logical line |
| `f{char}`, `F{char}` | Find a character forward or backward |
| `t{char}`, `T{char}` | Move just before or after that character |
| `;`, `,` | Repeat the last find in its original or opposite direction |
| `s` | Start Leap again |
| `v` | Toggle rectangular selection |
| Ctrl+V | Toggle character selection |
| `V` | Toggle whole-line selection |
| `y` | Copy selection and return to live display |
| `yy`, `yw`, `y$` | Copy a line or motion range and return live |
| Esc | Cancel pending search/motion, clear selection, then leave Vim mode |

Counts apply to motions and yanks: `3w`, `5j`, `y2w`, and `3yy`.
Vertical motion uses display rows and remembers the desired column.
Horizontal and logical-line motions respect soft wraps. Character and line
copies join soft-wrapped rows; rectangular copies retain row separators.
Wide characters stay intact at cursor and selection boundaries.

Character finds are case-sensitive and stay within the current pane's
logical line, including soft wraps. They work in normal navigation, all
selection modes, and yanks (`yfx`, `ytx`). Counts choose an occurrence
(`2fx`); an absent occurrence leaves the cursor and selection unchanged.
Esc cancels a pending find while retaining the selection. `;` and `,` skip
an adjacent till target so repeated `t`/`T` motions keep moving.

`k` and `j` always navigate; `kj` has no exit binding.
Within Leap, they remain search characters or target labels.

These mappings are personal: `v` selects a block, Ctrl+V selects characters,
and `H`/`L` act as `^`/`$`. Editing commands such as `d` have no action.
Unrecognized keys, paste shortcuts, and mouse input stay within Vim mode.
IME composition can supply Unicode Leap queries. Copy failure retains the
mode so the selection can be retried.

## Snapshot lifetime

Pane text is captured on demand through the existing local default-server
tmux integration. The captured text must match the frozen display, with
complete coverage of supported visible panes. In tmux copy mode, capture
uses the visible scrolled content; leaving Vim mode preserves tmux copy
mode and its scroll position. A changing or unsupported
viewport cancels entry; retry once the display settles. Custom sockets and
remote tmux sessions use the terminal grid, without pane-local metadata.

There are no background pane captures after entry. The idle tmux helper
closes after five seconds. Resizing or reloading configuration returns to
the live display so stale coordinates cannot be used. Exiting also restores
normal input routing; held modal keys remain consumed until released.

## Entry shortcut

Set the entry key in the existing `keyboard.bindings` list:

```yaml
keyboard:
  bindings:
    - {key: j, modifiers: [Control, Shift], action: VimMode}
```

Change `key` and `modifiers` to your preferred combination, then reload
with Ctrl+Shift+R. A configured `VimMode` action replaces the default entry
shortcut. Multiple entry bindings are supported; later bindings win on the
same key. Use `text: ""` on Ctrl+Shift+J to disable the default shortcut.
