# Filter, sort, and arrange columns

## Filter a column

Focus a column and press `f`, or choose **Filter column** in the command palette.
The popup contains a rule list, an operator, and a value field. Use `Tab` and
`Shift-Tab` to move between them.

1. Select the new-rule entry or an existing rule.
2. Focus the operator and change it with arrow keys or `Space`.
3. Focus the value field, type a value, and press `Enter` to apply the rule.
4. Press `Esc` to close the popup when finished.

Changes to the rule list apply as you make them. In that list, `Space` enables
or disables a rule; `Delete` or `Backspace` removes it. Invalid values and
regular expressions show an error for correction.

| Operator | Matching behavior |
| --- | --- |
| `<` | Less than the typed value |
| `>` | Greater than the typed value |
| `==` | Equal to the typed value |
| `contains` | Literal substring; `%` and `_` are ordinary characters |
| `regexp` | Matches a regular expression, such as `^(draft|pending)$` |

Comparison values are interpreted using the column's declared type. `contains`
uses SQLite's `LIKE` matching, which is case-insensitive for ASCII letters.
Regular expressions are case-sensitive unless you use a flag such as `(?i)`.

Multiple enabled rules in **one column** are alternatives (OR). Filters on
**different columns** must all match (AND). For example, `status == draft` and
`status == pending`, combined with `amount > 100`, mean:

```text
(status is draft OR pending) AND amount is greater than 100
```

Press `F` in the grid or choose **Clear filters** to clear every column filter.

## Sort rows

Press `s` on a column to cycle through ascending, descending, and unsorted.
Clicking a header also changes sorting. `S` adds the focused column as another
sort key, letting you group by one column and order within each group by another.

When the primary sort is a text column, press `'` followed by a letter to jump
to that letter. You can also click a letter in the alphabet rail when it is shown.

## Arrange columns

| Control | Result |
| --- | --- |
| `<` / `>` | Narrow / widen the focused column |
| `-` | Hide the focused column |
| **Show hidden columns** in `Ctrl-P` | Choose a hidden column to restore |
| **Freeze / unfreeze first column** in `Ctrl-P` | Keep the first column visible while scrolling horizontally |

Hiding a column affects the grid presentation. Exports and row copies still
include all table columns. Your arrangement, filters, and sorts are
[saved automatically](configuration.md#saved-table-views).
