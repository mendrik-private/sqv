//! Every key and mouse binding, grouped by workflow. The help popup and the
//! README key tables are generated from this list, so documentation cannot
//! drift from it; `App` input handling implements exactly these bindings.

pub struct Binding {
    pub keys: &'static str,
    pub action: &'static str,
}

pub struct Section {
    pub title: &'static str,
    pub bindings: &'static [Binding],
}

const fn b(keys: &'static str, action: &'static str) -> Binding {
    Binding { keys, action }
}

pub const SECTIONS: &[Section] = &[
    Section {
        title: "Move",
        bindings: &[
            b("↑↓←→ / h j k l", "Move between cells"),
            b("Home / End", "First / last column"),
            b("Ctrl-Home / Ctrl-End", "First / last cell of the table"),
            b("PgUp / PgDn", "Scroll one page"),
            b("Ctrl-↑ / Ctrl-↓", "Scroll one page"),
            b("Ctrl-G", "Go to row number"),
            b("' then a letter", "Jump to letter in a text-sorted column"),
        ],
    },
    Section {
        title: "Select & copy",
        bindings: &[
            b("Shift-↑ / Shift-↓", "Extend the row selection"),
            b("Space", "Toggle the focused row"),
            b("Ctrl-A", "Select all rows"),
            b("Esc", "Clear the selection, then focus the sidebar"),
            b("y / Ctrl-C", "Copy the focused cell"),
            b("Y", "Copy the focused or selected rows as JSON"),
        ],
    },
    Section {
        title: "Edit",
        bindings: &[
            b("Enter", "Edit the cell with the matching picker"),
            b("e", "Edit the value as text"),
            b("n", "Set the cell to NULL"),
            b("i / Ins", "Insert a row below"),
            b("d / Del", "Delete the focused or selected rows"),
            b("Ctrl-Z", "Undo the last write"),
        ],
    },
    Section {
        title: "Inspect",
        bindings: &[
            b("v", "Show the focused row as a record"),
            b("j", "Follow the link on a foreign-key cell"),
            b("r", "Rows in other tables that reference this row"),
            b("Backspace", "Go back after following a link"),
            b("Ctrl-F", "Find rows in this table"),
            b(":", "SQL console"),
        ],
    },
    Section {
        title: "Filter, sort & columns",
        bindings: &[
            b("f", "Filter the focused column"),
            b("F", "Clear all filters"),
            b("s", "Sort by the focused column (asc, desc, off)"),
            b("S", "Add the focused column as a further sort key"),
            b("< / >", "Narrow / widen the focused column"),
            b("-", "Hide the focused column"),
        ],
    },
    Section {
        title: "Tabs & panels",
        bindings: &[
            b("Tab / Shift-Tab", "Switch focus between sidebar and table"),
            b("Ctrl-B", "Show / hide the sidebar"),
            b("1-9 / 0", "Go to tab 1-10"),
            b("] / [ / Ctrl-PgDn / Ctrl-PgUp", "Next / previous tab"),
            b("Ctrl-W", "Close the current tab"),
        ],
    },
    Section {
        title: "Sidebar",
        bindings: &[
            b("↑↓ / j k", "Move"),
            b("← → / h l", "Collapse / expand a section"),
            b("Enter", "Open a table or view, fold a section"),
            b("i", "Show the schema of the selected item"),
            b("Esc", "Back to the table"),
        ],
    },
    Section {
        title: "Popups",
        bindings: &[
            b("Esc", "Close"),
            b("Enter", "Confirm the selection"),
            b("↑↓ PgUp PgDn Home End", "Move in lists"),
            b("typing", "Filter the list or edit the field"),
            b("Ctrl-A / Ctrl-E", "Start / end of the input"),
            b("Ctrl-U / Ctrl-W", "Delete to start / previous word"),
            b("Alt-Enter", "New line in the editor; save a staged row"),
            b("y / n", "Confirm / cancel a deletion"),
        ],
    },
    Section {
        title: "Mouse",
        bindings: &[
            b("Wheel", "Scroll the panel or list under the pointer"),
            b("Shift-wheel", "Scroll table columns"),
            b("Click", "Focus a cell, select a list item"),
            b("Click header", "Sort by that column"),
            b("Click / Ctrl-click gutter", "Select / toggle rows"),
            b("Drag scrollbar", "Scroll"),
            b("Click rail letter", "Jump to that letter"),
            b("Click / middle-click tab", "Activate / close the tab"),
        ],
    },
    Section {
        title: "App",
        bindings: &[
            b(
                "Ctrl-P",
                "Command palette: export, copy as CSV/SQL, search all tables, columns",
            ),
            b("?", "This help"),
            b("Ctrl-Q", "Quit"),
        ],
    },
];

/// The bindings as README markdown tables.
#[cfg(test)]
pub fn markdown() -> String {
    let mut out = String::new();
    for section in SECTIONS {
        out.push_str(&format!(
            "### {}\n\n| Keys | Action |\n| --- | --- |\n",
            section.title
        ));
        for binding in section.bindings {
            out.push_str(&format!("| `{}` | {} |\n", binding.keys, binding.action));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    const START: &str = "<!-- keymap:start -->\n";
    const END: &str = "<!-- keymap:end -->";

    /// Fails when README key tables differ from the key map. Run with
    /// `SQVIEW_UPDATE_README=1 cargo test` to regenerate them.
    #[test]
    fn readme_key_tables_match_the_keymap() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/README.md");
        let readme = std::fs::read_to_string(path).expect("read README");
        let start = readme.find(START).expect("README keymap start marker") + START.len();
        let end = readme.find(END).expect("README keymap end marker");
        let expected = format!("\n{}", super::markdown());
        if readme[start..end] != expected {
            if std::env::var_os("SQVIEW_UPDATE_README").is_some() {
                let updated = format!("{}{}{}", &readme[..start], expected, &readme[end..]);
                std::fs::write(path, updated).expect("write README");
            } else {
                panic!("README key tables are stale; run SQVIEW_UPDATE_README=1 cargo test");
            }
        }
    }
}
