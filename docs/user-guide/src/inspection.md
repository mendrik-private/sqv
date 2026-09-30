# Inspect records, schema, and links

## Read a whole record

Focus a row and press `v`, or choose **Show row as record** in the command
palette. The popup shows fields vertically and the selected value in full,
which is useful for wide tables and long text.

Move between fields with arrow keys. `y` copies the selected value. `Enter`
opens its editor when the table is editable. `Esc` or `v` closes the record.

When a text value contains a JSON object or array, `o` opens a tree viewer.
`Enter` folds a node, `←`/`→` collapse or expand it, and `y` copies the selected
node. `Esc` returns to the record.

## Inspect schema and indexes

In the sidebar, select a table, view, or index and press `i`. You can also choose
**Show schema** from the command palette for the active table. The schema popup
shows the SQL definition and available column, key, or index information.

Choose **Reload schema** from the palette after an external schema change if
the sidebar needs refreshing, especially when using `--no-watch`.

## Follow relationships

A foreign-key cell links to a referenced row. Focus it and press `j`, or select
the linked field in record view and press `j`. `Backspace` returns after following
a link.

To go in the other direction, focus a row and press `r` or choose **Show
referencing rows**. Select a relationship and press `Enter`. sqview opens the
referencing table with an equality filter for the current key value.

That filter is saved like other table filters. Press `F` in the destination
table to see all its rows again. `Backspace` navigates back.

Relationships come from declared SQLite foreign keys. A column containing an
ID without a foreign-key declaration does not automatically become a link.
