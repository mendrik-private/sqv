# Browse tables and tabs

The sidebar groups tables, views, and indexes. The grid shows the active table
or view. Tabs sit above the grid, and the status bar shows the current position
and application state, including read-only mode.

## Open tables and move between panels

Select an item in the sidebar with `↑`/`↓` or `j`/`k`. Use `←`/`→` or `h`/`l`
to collapse or expand groups. `Enter` opens a table or view. The command palette
(`Ctrl-P`) also offers **Open table: …** commands.

`Tab` and `Shift-Tab` switch focus between sidebar and grid. `Ctrl-B` hides or
shows the sidebar to give wide tables more room. In the sidebar, `Esc` returns
focus to the table; in the grid, `Esc` clears the row selection first, then
returns focus to the sidebar.

## Navigate the grid

| Keys | Destination |
| --- | --- |
| Arrow keys or `h j k l` | Adjacent cell |
| `Home` / `End` | First / last column |
| `Ctrl-Home` / `Ctrl-End` | First / last cell of the current table view |
| `PgUp` / `PgDn` or `Ctrl-↑` / `Ctrl-↓` | One page up / down |
| `Ctrl-G` | A row number in the current filtered and sorted view |

On a foreign-key cell, `j` [follows the link](inspection.md#follow-relationships)
instead of moving down. Use `↓` when you want to keep browsing that column.

sqview loads rows in windows as you scroll, so browsing does not require loading
the entire table into the grid. Filters and sorts can still take time on large
databases.

## Keep multiple tables open

Opening a table creates or activates its tab. Tabs retain their grid position
when you switch away and return.

- `1`–`9` select tabs 1–9; `0` selects tab 10.
- `]` / `[` or `Ctrl-PgDn` / `Ctrl-PgUp` move to the next / previous tab.
- `Ctrl-W` closes the current tab.
- Click a tab to activate it; middle-click or click its close button to close it.

Filters, sort keys, widths, hidden columns, and the frozen first column are
[saved per database and table](configuration.md#saved-table-views).

## Use the mouse

Click a cell to focus it, and click a column header to sort. Scroll the panel
under the pointer with the wheel; `Shift-wheel` scrolls columns. Scrollbars can
be dragged. Click the row gutter to select a row or `Ctrl-click` it to toggle
rows in a selection.

Use [filtering and sorting](filtering-and-sorting.md) to shape a table, or
[selection and copying](copy-and-export.md) to collect rows.
