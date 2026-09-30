# sqview

**sqview** is a keyboard-first terminal viewer for SQLite databases. It is built for fast table browsing, in-place editing, filtering, sorting, and foreign-key navigation without leaving the terminal.

<img width="2874" height="1593" alt="image" src="https://github.com/user-attachments/assets/c4e6ffbb-f28f-40fe-892c-0abf9f2fc5d0" />
<br/>
<img width="2874" height="1593" alt="image" src="https://github.com/user-attachments/assets/43446fd9-01b1-4856-999a-2a59e59fb183" />

[Read the sqview User Guide](https://mendrik-private.github.io/sqv/) for installation, browsing, editing, filtering, SQL, export, and troubleshooting.

## Highlights

- Fast virtual scrolling for large tables and views
- Rich cell editors for text, enums, dates, datetimes, and foreign keys
- Per-column filters, multi-column sorting, and alphabet jump navigation
- Tabs that keep their position, and saved per-table filters, sort order, hidden, resized and frozen columns
- Row detail view with a JSON tree viewer, schema/DDL and index inspection
- Foreign-key links both ways: follow a link, or list the rows that reference the current one
- Find in a table, search across all tables, go to a row number
- SQL console with a persistent history
- Export views or selections to CSV, JSON, or SQL; copy rows as JSON, CSV or SQL inserts
- Read-only mode for safe inspection, shown in the status bar

## Install

### Debian/Ubuntu package

Download the latest Debian package from GitHub Releases and install it directly:

```bash
tmp_deb="$(mktemp /tmp/sqview-linux-amd64.XXXXXX.deb)"
curl -fL --retry 5 --retry-all-errors --retry-delay 2 -o "$tmp_deb" \
  https://github.com/mendrik-private/sqv/releases/latest/download/sqview-linux-amd64.deb
chmod 0644 "$tmp_deb"
sudo apt install "$tmp_deb"
rm -f "$tmp_deb"
```

This installs the `sqview` package and the `sqview` command, avoiding the `sqv` name collision with Ubuntu's OpenPGP tool.

### Release binary

Download the latest Linux binary archive from GitHub Releases and install it into `/usr/local/bin`:

```bash
curl -fL --retry 5 --retry-all-errors --retry-delay 2 -o sqview-linux-x86_64.tar.gz \
  https://github.com/mendrik-private/sqv/releases/latest/download/sqview-linux-x86_64.tar.gz
tar -xzf sqview-linux-x86_64.tar.gz
sudo install -m 0755 sqview /usr/local/bin/sqview
```

### From source

```bash
cargo install --path . --bin sqview
```

Or run directly from the checkout:

```bash
cargo run --release -- path/to/database.db
```

Use the built-in help to inspect the current CLI surface:

```bash
sqview --help
sqview --version
```

## Usage

```text
sqview [OPTIONS] <DB_PATH>
sqview check-terminal
sqview paths
```

- `DB_PATH`: path to a SQLite database, or `:memory:`
- `--readonly`: disable writes
- `--no-watch`: disable automatic external refresh when the database file changes
- `check-terminal`: print detected terminal capabilities
- `paths`: print the config, data, saved-view and SQL-history paths used by sqview

## Keybindings

Press `?` in the app for the same list. Every command is also in the command palette (`Ctrl-P`),
which shows its key next to it.

<!-- ANCHOR: keymap -->
<!-- keymap:start -->

### Move

| Keys | Action |
| --- | --- |
| `↑↓←→ / h j k l` | Move between cells |
| `Home / End` | First / last column |
| `Ctrl-Home / Ctrl-End` | First / last cell of the table |
| `PgUp / PgDn` | Scroll one page |
| `Ctrl-↑ / Ctrl-↓` | Scroll one page |
| `Ctrl-G` | Go to row number |
| `' then a letter` | Jump to letter in a text-sorted column |

### Select & copy

| Keys | Action |
| --- | --- |
| `Shift-↑ / Shift-↓` | Extend the row selection |
| `Space` | Toggle the focused row |
| `Ctrl-A` | Select all rows |
| `Esc` | Clear the selection, then focus the sidebar |
| `y / Ctrl-C` | Copy the focused cell |
| `Y` | Copy the focused or selected rows as JSON |

### Edit

| Keys | Action |
| --- | --- |
| `Enter` | Edit the cell with the matching picker |
| `e` | Edit the value as text |
| `n` | Set the cell to NULL |
| `i / Ins` | Insert a row below |
| `d / Del` | Delete the focused or selected rows |
| `Ctrl-Z` | Undo the last write |

### Inspect

| Keys | Action |
| --- | --- |
| `v` | Show the focused row as a record |
| `j` | Follow the link on a foreign-key cell |
| `r` | Rows in other tables that reference this row |
| `Backspace` | Go back after following a link |
| `Ctrl-F` | Find rows in this table |
| `:` | SQL console |

### Filter, sort & columns

| Keys | Action |
| --- | --- |
| `f` | Filter the focused column |
| `F` | Clear all filters |
| `s` | Sort by the focused column (asc, desc, off) |
| `S` | Add the focused column as a further sort key |
| `< / >` | Narrow / widen the focused column |
| `-` | Hide the focused column |

### Tabs & panels

| Keys | Action |
| --- | --- |
| `Tab / Shift-Tab` | Switch focus between sidebar and table |
| `Ctrl-B` | Show / hide the sidebar |
| `1-9 / 0` | Go to tab 1-10 |
| `] / [ / Ctrl-PgDn / Ctrl-PgUp` | Next / previous tab |
| `Ctrl-W` | Close the current tab |

### Sidebar

| Keys | Action |
| --- | --- |
| `↑↓ / j k` | Move |
| `← → / h l` | Collapse / expand a section |
| `Enter` | Open a table or view, fold a section |
| `i` | Show the schema of the selected item |
| `Esc` | Back to the table |

### Popups

| Keys | Action |
| --- | --- |
| `Esc` | Close |
| `Enter` | Confirm the selection |
| `↑↓ PgUp PgDn Home End` | Move in lists |
| `typing` | Filter the list or edit the field |
| `Ctrl-A / Ctrl-E` | Start / end of the input |
| `Ctrl-U / Ctrl-W` | Delete to start / previous word |
| `Alt-Enter` | New line in the editor; save a staged row |
| `y / n` | Confirm / cancel a deletion |

### Mouse

| Keys | Action |
| --- | --- |
| `Wheel` | Scroll the panel or list under the pointer |
| `Shift-wheel` | Scroll table columns |
| `Click` | Focus a cell, select a list item |
| `Click header` | Sort by that column |
| `Click / Ctrl-click gutter` | Select / toggle rows |
| `Drag scrollbar` | Scroll |
| `Click rail letter` | Jump to that letter |
| `Click / middle-click tab` | Activate / close the tab |

### App

| Keys | Action |
| --- | --- |
| `Ctrl-P` | Command palette: export, copy as CSV/SQL, search all tables, columns |
| `?` | This help |
| `Ctrl-Q` | Quit |

<!-- keymap:end -->
<!-- ANCHOR_END: keymap -->

## Configuration

Configuration is read from:

```text
$XDG_CONFIG_HOME/sqview/config.toml
```

On first launch, sqview creates this file automatically if it is missing.

Example:

```toml
nerd_font = true

[theme]
accent = "#d99a5e"
bg = "#23211f"

[symbols]
table_icon = "󰓫"
view_icon = "󰈈"
index_icon = "󰓹"
filter_icon = "󰈲"
selection = "⏵"
tab_close = "×"
```

Every theme token and every UI glyph/icon now lives in this config file, so users can fully restyle the palette and override the icon set without editing Rust sources. Single-cell drawing symbols such as borders, cursors, and selection markers must stay one character wide.

## Development

### Local validation

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
```

### Release pipeline

GitHub Actions provides:

1. **CI** on pushes and pull requests: format check, clippy, and tests
2. **Tagged releases** on `v*` tags:
    - build the release binary
    - build a Debian package
    - publish GitHub Release assets
3. **Manual releases** from the Actions tab:
    - use the selected branch's `Cargo.toml` version as the release tag
    - create or update the matching GitHub Release for that commit

To cut a release:

```bash
git tag -a v0.2.6 -m 'v0.2.6'
git push origin v0.2.6
```
