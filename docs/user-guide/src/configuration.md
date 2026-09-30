# Configuration and saved views

Run this command to locate the files sqview uses on your platform:

```bash
sqview paths
```

It prints `config`, `data`, `views`, and `history` paths. On Linux the config
normally lives at `$XDG_CONFIG_HOME/sqview/config.toml`, or
`~/.config/sqview/config.toml` when `XDG_CONFIG_HOME` is unset. Use the reported
paths rather than assuming the same directories on every platform.

## Theme and symbols

The first database session creates a configuration file if it is missing.
That file lists the default colors and glyphs. Edit it and restart sqview to
apply changes. Omitted settings retain defaults.

```toml
nerd_font = false

[theme]
accent = "#d99a5e"
bg = "#23211f"

[symbols]
table_icon = "[T]"
view_icon = "[V]"
selection = ">"
tab_close = "x"
```

`nerd_font = true` uses the default Nerd Font icons; `false` selects text-based
alternatives. Symbol overrides still apply to the chosen set. Theme colors use
hexadecimal RGB values such as `#d99a5e`.

Drawing symbols such as borders, cursors, and selection markers must be single
characters occupying one terminal cell. Icon strings can contain multiple
characters. Invalid configuration or symbol values produce a startup error.

## Saved table views

sqview automatically saves filters, sort keys, hidden columns, column widths,
and whether the first column is frozen. Settings are stored per database and
table in the `views` directory reported by `sqview paths`.

Opening a table restores its saved settings. Closing a tab does not clear them.
If rows appear to be missing, press `F` to clear restored filters. To reset all
saved views, quit sqview and move the reported `views` directory aside before
reopening the application.

Tab positions last while tabs stay open in the current session. Saved view
settings do not reopen your previous session's tabs.

## External changes and history

By default, sqview watches the database and its WAL/shared-memory companions
for external changes and refreshes the workspace. `--no-watch` disables this
watcher. The in-memory database is not watched.

SQL statements are saved separately in the `history` file. Grid undo history is
session-only and is separate from SQL statement history.
