# Run SQL

Press `:` or choose **SQL console** from the command palette. Type a statement
and press `Enter` to execute it. Query results appear below the input; write
statements report the number of changed rows. Errors appear in the console.

For example, replace the table and column names with ones in your database:

```sql
SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name;
```

The console displays up to 1,000 result rows and indicates when results are
truncated. Add a `WHERE` clause or a `LIMIT` for a focused query. Arrow keys and
`PgUp`/`PgDn` scroll the results; `Ctrl-←`/`Ctrl-→` scroll columns. `Esc` closes
the console.

`Ctrl-P` and `Ctrl-N` inside the console browse previous and next statements.
These keys control history while the console is open. History is saved in the
SQL-history file reported by `sqview paths`.

## Writes and schema changes

In a writable session, SQL can change data and schema. Successful write
statements request a refresh of the database workspace. In read-only mode,
writes are refused. SQL writes are outside the grid's undo history.

## Try a temporary database

Run `sqview :memory:`, open the console, and execute each statement separately:

```sql
CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL);
```

```sql
INSERT INTO notes (title) VALUES ('First note'), ('Second note');
```

```sql
SELECT * FROM notes;
```

Close the console and open `notes` from the sidebar. If needed, use **Reload
schema** in the command palette. This database is temporary and disappears
when sqview exits.
