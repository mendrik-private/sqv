# Install and get started

## Debian or Ubuntu

Download and install the latest Linux amd64 package:

```bash
tmp_deb="$(mktemp /tmp/sqview-linux-amd64.XXXXXX.deb)"
curl -fL --retry 5 --retry-all-errors --retry-delay 2 -o "$tmp_deb" \
  https://github.com/mendrik-private/sqv/releases/latest/download/sqview-linux-amd64.deb
chmod 0644 "$tmp_deb"
sudo apt install "$tmp_deb"
rm -f "$tmp_deb"
```

The package installs `sqview`. Ubuntu also has an unrelated OpenPGP tool named
`sqv`, so use `sqview` to launch this application.

## Linux release binary

```bash
curl -fL --retry 5 --retry-all-errors --retry-delay 2 -o sqview-linux-x86_64.tar.gz \
  https://github.com/mendrik-private/sqv/releases/latest/download/sqview-linux-x86_64.tar.gz
tar -xzf sqview-linux-x86_64.tar.gz
sudo install -m 0755 sqview /usr/local/bin/sqview
```

See [GitHub Releases](https://github.com/mendrik-private/sqv/releases) for the
available versions and assets.

## Build from source

Install a stable Rust toolchain, then clone and install the application:

```bash
git clone https://github.com/mendrik-private/sqv.git
cd sqv
cargo install --path . --bin sqview
```

You can also run directly from the checkout:

```bash
cargo run --release -- path/to/database.db
```

## Open a database

```text
sqview [OPTIONS] <DB_PATH>
sqview check-terminal
sqview paths
```

| Argument or option | What it does |
| --- | --- |
| `DB_PATH` | Opens an existing SQLite database file; quote paths containing spaces. |
| `:memory:` | Opens a temporary in-memory database for this session. |
| `--readonly` | Disables database writes for the entire session. |
| `--no-watch` | Disables automatic refresh when another program changes the database files. |
| `--help` | Prints usage, options, and examples. |
| `--version` | Prints the installed version. |
| `check-terminal` | Prints terminal capability hints without opening a database. |
| `paths` | Prints configuration, data, saved-view, and SQL-history locations. |

An ordinary database path must already exist. To try sqview without a file,
run `sqview :memory:` and use the [SQL console](sql-console.md) to create a table.
An in-memory database disappears when you quit.

## Your first session

1. Run `sqview path/to/database.db --readonly`.
2. In the sidebar, select a table with `↑` and `↓`, then press `Enter`.
3. Move through cells with the arrow keys. Press `v` to inspect a whole record.
4. Press `Esc` to close the record, then `Ctrl-F` to find a row.
5. Press `Ctrl-P`, type a command name, and press `Enter` to run it.
6. Press `Ctrl-Q` to quit.

Use [browsing](browsing.md) to learn tabs, panels, and large-table navigation.
