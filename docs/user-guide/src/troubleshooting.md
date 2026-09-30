# Troubleshooting

## The sqv command opens an unrelated tool

Launch `sqview`. The repository is named sqv, but the binary and Debian package
are sqview to avoid the OpenPGP tool's name collision.

## The database path is rejected

sqview requires an existing file, except for `:memory:`. Check the path and
quote it if it contains spaces:

```bash
sqview "path/to/my database.db" --readonly
```

If an existing file fails to open, check that it is SQLite and that you have
the required file and directory permissions.

## A table looks empty or a row cannot be found

Press `F` to clear saved filters. Row numbers belong to the current filtered
and sorted view; use a filter or SQL for a primary-key lookup. Search result
lists are capped, so narrow your term if a broad search omits the desired row.

## Editing is disabled

Check the read-only indicator. A session started with `--readonly` must be
restarted without it to enable writes. **Toggle read-only** applies to writable
sessions. SQLite views, generated columns, and tables without a usable rowid
cannot be edited through the grid. Constraint errors must be corrected before
a write can succeed.

## Icons or colors look wrong

Run `sqview check-terminal` for environment-based capability hints. It cannot
detect whether your font supplies Nerd Font glyphs. Set `nerd_font = false`
in the [configuration file](configuration.md) and restart if icons are missing.
For the full palette, use a terminal configured for truecolor.

If a config edit prevents startup, use `sqview paths` to find the file, correct
the reported setting, or move the file aside so the next launch creates defaults.

## Alt-Enter or another shortcut is intercepted

Check terminal or window-manager shortcuts. sqview enables enhanced keyboard
reporting when the terminal supports it; some terminal configurations still
intercept key combinations. Popup footers show the expected binding. Use the
command palette for commands with an alternative there.

## Clipboard copy does not reach the desktop

sqview uses the terminal clipboard protocol (OSC 52). Your terminal, SSH setup,
or multiplexer must allow clipboard integration. If it blocks the operation,
use [file export](copy-and-export.md#export-a-view-or-selection) to retrieve data.

## External changes do not appear

Check whether you started with `--no-watch`. Choose **Reload schema** from the
palette and reopen the table as needed. File watching covers the database and
its WAL/shared-memory files; it is not enabled for `:memory:`.

## Export fails or overwrites an earlier file

Choose a writable destination in an existing directory. Export replaces an
existing file at the chosen path; use a new filename to retain both versions.
JSON export represents BLOBs as `null`, so use a database backup for complete
binary-data preservation.

## Report a problem

Open an [issue on GitHub](https://github.com/mendrik-private/sqv/issues) with:

- `sqview --version` and your operating system;
- your terminal and the output of `sqview check-terminal`;
- what you did, what happened, and the expected behavior;
- a small reproduction database or SQL example, when possible.

Use a minimal sample that you are comfortable sharing publicly.
