# Copy and export

## Select rows

`Space` toggles the focused row. `Shift-↑`/`Shift-↓` extend a selection, and
`Ctrl-A` selects all rows in the current filtered view. `Esc` clears the selection.
The mouse offers row-gutter selection and `Ctrl-click` to toggle individual rows.

A selection can include rows outside the visible page. Filters determine the
view that selection and whole-view export operate on.

## Copy to the clipboard

| Command | Content |
| --- | --- |
| `y` or `Ctrl-C` in the grid | Focused cell |
| `Y` or **Copy rows as JSON** | Selected rows, or the focused row if there is no selection |
| **Copy rows as CSV** | Selected rows or focused row, including a header |
| **Copy rows as SQL inserts** | Selected rows or focused row as `INSERT` statements |
| **Copy column values** | Focused column for the selection, or the whole current view if nothing is selected |

Commands without a direct key are in `Ctrl-P`. Row copies include all columns,
including hidden ones. Clipboard support depends on the terminal's clipboard
integration; see [troubleshooting](troubleshooting.md#clipboard-copy-does-not-reach-the-desktop).

## Export a view or selection

1. Open `Ctrl-P` and choose **Export as CSV…**, **Export as JSON…**, or
   **Export as SQL…**.
2. Edit the destination path. The default suggests a timestamped file in your
   home directory.
3. If rows are selected, use `Tab` to switch between the selection and the whole
   current view. Selection-only is the default when a selection exists.
4. Press `Enter` and wait for the completion message, or `Esc` to cancel the popup.

Whole-view export includes all rows matching your filters in the current sort
order, including rows that are not loaded on screen. Export includes hidden
columns. It replaces an existing destination file after successfully writing
the export; choose a new path if you want to keep an earlier file.

| Format | Output |
| --- | --- |
| CSV | Header row followed by data rows; SQL `NULL` becomes an empty field |
| JSON | Array of objects keyed by column name; SQL `NULL` becomes JSON `null` |
| SQL | `INSERT` statements with quoted table and column names; no schema definition |

BLOBs become JSON `null` and are not preserved by JSON export. Use an SQLite
backup when you need a complete database copy with schema and binary data.
Export also works in read-only mode because it writes an output file without
changing the database.
