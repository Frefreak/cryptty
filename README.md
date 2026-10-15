A vibe-coded personal terminal that suits my needs.

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
