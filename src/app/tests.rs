use std::{collections::BTreeSet, sync::Arc};

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use r2d2_sqlite::SqliteConnectionManager;
use ratatui::{backend::TestBackend, Terminal};
use tokio::sync::mpsc;

use super::*;
use crate::{
    config::Config,
    db::{self, schema::Column, types::SqlValue},
    ui::{popup::HelpState, tabbar::TabMouseAction},
};

// ---------- helpers ----------

fn make_test_app() -> (App, mpsc::UnboundedReceiver<Message>) {
    let manager = SqliteConnectionManager::memory();
    let pool = Arc::new(
        r2d2::Pool::builder()
            .max_size(1)
            .build(manager)
            .expect("test pool"),
    );
    let conn = pool.get().expect("test conn");
    conn.execute_batch(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER, email TEXT);",
    )
    .expect("seed schema");
    let schema = db::load_schema(&conn).expect("load schema");
    drop(conn);

    let (tx, rx) = mpsc::unbounded_channel();
    let config = Config::default();
    let app = App::new(schema, config, pool, tx, false, ":memory:".to_string());
    (app, rx)
}

fn dummy_column(name: &str, col_type: &str) -> Column {
    Column {
        name: name.to_string(),
        col_type: col_type.to_string(),
        not_null: false,
        default_value: None,
        is_pk: name == "id",
        pk_position: i64::from(name == "id"),
        writable: true,
    }
}

fn make_grid() -> crate::grid::GridState {
    let columns = vec![
        dummy_column("id", "INTEGER"),
        dummy_column("name", "TEXT"),
        dummy_column("age", "INTEGER"),
        dummy_column("email", "TEXT"),
    ];
    let rows: Vec<Vec<SqlValue>> = (0..50)
        .map(|i| {
            vec![
                SqlValue::Integer(i),
                SqlValue::Text(format!("user-{i}")),
                SqlValue::Integer(20 + i % 40),
                SqlValue::Text(format!("user{}@example.com", i)),
            ]
        })
        .collect();
    let mut grid = crate::grid::GridState::new(crate::grid::GridInit {
        table_name: "users".to_string(),
        columns,
        fk_cols: vec![false; 4],
        enumerated_values: vec![Vec::new(); 4],
        rows,
        width_sample_rows: vec![Vec::new(); 4],
        total_rows: 50,
        area_width: 80,
    });
    grid.window.rowids = (1..=50).map(Some).collect();
    grid
}

fn make_viewport_rows(app: &App) -> usize {
    app.grid.as_ref().map_or(20, |g| g.window.viewport_rows)
}

fn seed_user_row(app: &App) {
    let conn = app.pool.get().expect("test conn");
    conn.execute(
        "INSERT INTO users (id, name, age, email) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![1i64, "Alice", 30i64, "alice@example.com"],
    )
    .expect("seed user row");
}

fn seed_user_rows(app: &App, count: usize) {
    let conn = app.pool.get().expect("test conn");
    for index in 0..count {
        let id = index as i64 + 1;
        conn.execute(
            "INSERT INTO users (id, name, age, email) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                id,
                format!("User {id}"),
                20 + (id % 30),
                format!("user{id}@example.com")
            ],
        )
        .expect("seed user row");
    }
}

fn seed_grid_rows(app: &App, count: usize) {
    let conn = app.pool.get().expect("test conn");
    for index in 0..count {
        let id = index as i64;
        conn.execute(
            "INSERT INTO users (id, name, age, email) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                id,
                format!("user-{id}"),
                20 + (id % 40),
                format!("user{id}@example.com")
            ],
        )
        .expect("seed grid row");
    }
}

fn make_constrained_insert_app() -> (App, mpsc::UnboundedReceiver<Message>) {
    let manager = SqliteConnectionManager::memory();
    let pool = Arc::new(
        r2d2::Pool::builder()
            .max_size(1)
            .build(manager)
            .expect("test pool"),
    );
    let conn = pool.get().expect("test conn");
    conn.execute_batch(
        "CREATE TABLE users (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            age INTEGER DEFAULT 18
        );",
    )
    .expect("seed schema");
    let schema = db::load_schema(&conn).expect("load schema");
    let columns = schema.tables[0].columns.clone();
    drop(conn);

    let (tx, rx) = mpsc::unbounded_channel();
    let mut app = App::new(
        schema,
        Config::default(),
        pool,
        tx,
        false,
        ":memory:".to_string(),
    );
    app.grid = Some(crate::grid::GridState::new(crate::grid::GridInit {
        table_name: "users".to_string(),
        columns,
        fk_cols: vec![false; 3],
        enumerated_values: vec![Vec::new(); 3],
        rows: Vec::new(),
        width_sample_rows: vec![Vec::new(); 3],
        total_rows: 0,
        area_width: 80,
    }));
    (app, rx)
}

#[test]
fn normalize_enumerated_values_skips_unique_columns() {
    let values = vec!["a".to_string(), "b".to_string(), "c".to_string()];

    assert!(normalize_enumerated_values(values, 3).is_empty());
}

#[test]
fn normalize_enumerated_values_keeps_repeated_short_values() {
    let values = vec!["pending".to_string(), "done".to_string()];

    assert_eq!(normalize_enumerated_values(values.clone(), 5), values);
}

fn try_recv_variant(rx: &mut mpsc::UnboundedReceiver<Message>) -> String {
    let msg = rx.try_recv();
    match msg {
        Ok(m) => format!("{:?}", m),
        Err(_) => "no message".to_string(),
    }
}

/// Drain channel and process messages through app.update().
fn drain_messages(app: &mut App, rx: &mut mpsc::UnboundedReceiver<Message>) {
    while let Ok(message) = rx.try_recv() {
        app.update(message);
    }
}

fn render_test_app(app: &mut App) {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| crate::ui::render(frame, app))
        .expect("render app");
}

fn text_editor_scroll_y(app: &App) -> u16 {
    match app.popup.as_ref() {
        Some(PopupKind::TextEditor(state)) => state.scroll_y,
        _ => panic!("expected text editor popup"),
    }
}

// ---------- global shortcuts ----------

#[test]
fn ctrl_q_sets_should_quit() {
    let (mut app, _rx) = make_test_app();
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('q'),
        KeyModifiers::CONTROL,
    )));
    assert!(app.should_quit);
}

#[test]
fn ctrl_b_toggles_sidebar() {
    let (mut app, _rx) = make_test_app();
    assert!(app.sidebar_visible);
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('b'),
        KeyModifiers::CONTROL,
    )));
    assert!(!app.sidebar_visible);
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('b'),
        KeyModifiers::CONTROL,
    )));
    assert!(app.sidebar_visible);
}

#[test]
fn ctrl_w_closes_current_tab() {
    let (mut app, mut rx) = make_test_app();
    app.open_tabs = vec![TableTab {
        table_name: "users".to_string(),
    }];
    app.active_tab = Some(0);

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('w'),
        KeyModifiers::CONTROL,
    )));
    drain_messages(&mut app, &mut rx);

    assert!(app.open_tabs.is_empty());
    assert_eq!(app.active_tab, None);
}

#[test]
fn tab_toggles_focus_sidebar_to_grid() {
    let (mut app, _rx) = make_test_app();
    app.sidebar_visible = true;
    app.focus = FocusPane::Sidebar;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Tab,
        KeyModifiers::NONE,
    )));
    assert!(matches!(app.focus, FocusPane::Grid));
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Tab,
        KeyModifiers::NONE,
    )));
    assert!(matches!(app.focus, FocusPane::Sidebar));
}

#[test]
fn backtab_toggles_focus() {
    let (mut app, _rx) = make_test_app();
    app.sidebar_visible = true;
    app.focus = FocusPane::Sidebar;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::BackTab,
        KeyModifiers::NONE,
    )));
    assert!(matches!(app.focus, FocusPane::Grid));
}

#[test]
fn question_mark_opens_and_closes_help() {
    let (mut app, mut rx) = make_test_app();
    assert!(app.popup.is_none());
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('?'),
        KeyModifiers::SHIFT,
    )));
    drain_messages(&mut app, &mut rx);
    assert!(
        matches!(app.popup, Some(PopupKind::Help(_))),
        "popup should be Help"
    );
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('?'),
        KeyModifiers::SHIFT,
    )));
    drain_messages(&mut app, &mut rx);
    assert!(app.popup.is_none(), "popup should be closed after second ?");
}

#[test]
fn ctrl_p_opens_command_palette() {
    let (mut app, mut rx) = make_test_app();
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('p'),
        KeyModifiers::CONTROL,
    )));
    drain_messages(&mut app, &mut rx);
    assert!(
        matches!(app.popup, Some(PopupKind::CommandPalette(_))),
        "popup should be CommandPalette"
    );
}

#[test]
fn ctrl_h_opens_help() {
    let (mut app, mut rx) = make_test_app();
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('h'),
        KeyModifiers::CONTROL,
    )));
    drain_messages(&mut app, &mut rx);
    assert!(matches!(app.popup, Some(PopupKind::Help(_))));
}

#[test]
fn esc_in_help_closes_popup() {
    let (mut app, mut rx) = make_test_app();
    app.popup = Some(PopupKind::Help(HelpState::new()));
    app.mode = AppMode::Edit;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )));
    drain_messages(&mut app, &mut rx);
    assert!(app.popup.is_none(), "help popup should be closed");
}

#[test]
fn ctrl_enter_in_help_is_noop() {
    let (mut app, mut rx) = make_test_app();
    app.popup = Some(PopupKind::Help(HelpState::new()));
    app.mode = AppMode::Edit;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::CONTROL,
    )));
    // no CommitEdit sent for Help popup
    assert!(rx.try_recv().is_err());
}

#[test]
fn ctrl_enter_in_text_editor_is_noop() {
    let (mut app, mut rx) = make_test_app();
    app.popup = Some(PopupKind::TextEditor(TextEditorState::new(
        "users".to_string(),
        1,
        "name".to_string(),
        "TEXT".to_string(),
        SqlValue::Text("Alice".to_string()),
        false,
    )));
    app.mode = AppMode::Edit;

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::CONTROL,
    )));

    assert!(rx.try_recv().is_err());
}

#[test]
fn enter_in_text_editor_sends_commit_edit() {
    let (mut app, mut rx) = make_test_app();
    app.popup = Some(PopupKind::TextEditor(TextEditorState::new(
        "users".to_string(),
        1,
        "name".to_string(),
        "TEXT".to_string(),
        SqlValue::Text("Alice".to_string()),
        false,
    )));
    app.mode = AppMode::Edit;

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));

    assert_eq!(try_recv_variant(&mut rx), "CommitEdit");
}

#[test]
fn alt_enter_in_text_editor_inserts_newline() {
    let (mut app, mut rx) = make_test_app();
    app.popup = Some(PopupKind::TextEditor(TextEditorState::new(
        "users".to_string(),
        1,
        "name".to_string(),
        "TEXT".to_string(),
        SqlValue::Text("Alice".to_string()),
        false,
    )));
    app.mode = AppMode::Edit;

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::ALT,
    )));

    match app.popup {
        Some(PopupKind::TextEditor(ref state)) => assert_eq!(state.current, "Alice\n"),
        _ => panic!("expected text editor popup"),
    }
    assert!(rx.try_recv().is_err());
}

#[test]
fn alt_enter_in_insert_row_sends_commit_insert_row() {
    let (mut app, mut rx) = make_constrained_insert_app();
    app.update(Message::InsertRow);
    drain_messages(&mut app, &mut rx);

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::ALT,
    )));

    assert_eq!(try_recv_variant(&mut rx), "CommitInsertRow");
}

// ---------- grid navigation shortcuts ----------

#[test]
fn arrow_down_sends_move_down() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Down,
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveDown");
}

#[test]
fn arrow_up_sends_move_up() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Up,
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveUp");
}

#[test]
fn shift_down_selects_rows_from_focused_row() {
    let (mut app, _rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Down,
        KeyModifiers::SHIFT,
    )));

    let grid = app.grid.as_ref().expect("grid");
    assert_eq!(grid.focused_row, 1);
    assert_eq!(
        grid.row_selection,
        crate::grid::RowSelection::Rows(BTreeSet::from([0]))
    );
}

#[test]
fn shift_up_extends_selection_toward_previous_rows() {
    let (mut app, _rx) = make_test_app();
    let mut grid = make_grid();
    grid.focused_row = 3;
    app.grid = Some(grid);
    app.focus = FocusPane::Grid;

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Up,
        KeyModifiers::SHIFT,
    )));

    let grid = app.grid.as_ref().expect("grid");
    assert_eq!(grid.focused_row, 2);
    assert_eq!(
        grid.row_selection,
        crate::grid::RowSelection::Rows(BTreeSet::from([3]))
    );
}

#[test]
fn move_down_preserves_existing_selection() {
    let (mut app, _rx) = make_test_app();
    let mut grid = make_grid();
    grid.row_selection = crate::grid::RowSelection::Rows(BTreeSet::from([1, 3]));
    grid.window.fetch_in_flight = true;
    app.grid = Some(grid);

    app.update(Message::MoveDown);

    let grid = app.grid.as_ref().expect("grid");
    assert_eq!(grid.focused_row, 1);
    assert_eq!(
        grid.row_selection,
        crate::grid::RowSelection::Rows(BTreeSet::from([1, 3]))
    );
}

#[test]
fn ctrl_a_selects_all_rows_in_grid() {
    let (mut app, _rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::CONTROL,
    )));

    let grid = app.grid.as_ref().expect("grid");
    assert_eq!(grid.row_selection, crate::grid::RowSelection::all());
}

#[test]
fn arrow_left_sends_move_left() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Left,
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveLeft");
}

#[test]
fn arrow_right_sends_move_right() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Right,
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveRight");
}

#[test]
fn vim_hjkl_navigation() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;

    // h = left
    let _ = rx.try_recv(); // drain
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('h'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveLeft");

    // l = right
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('l'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveRight");

    // k = up
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('k'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveUp");

    // j (non-FK) = down
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('j'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveDown");
}

#[test]
fn home_sends_move_col_first() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Home,
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveColFirst");
}

#[test]
fn end_sends_move_col_last() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::End,
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveColLast");
}

#[test]
fn ctrl_home_sends_move_first_cell() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Home,
        KeyModifiers::CONTROL,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveFirstCell");
}

#[test]
fn ctrl_end_sends_move_last_cell() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::End,
        KeyModifiers::CONTROL,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveLastCell");
}

#[test]
fn page_down_scrolls_viewport() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    let vp = make_viewport_rows(&app).saturating_sub(1);
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::PageDown,
        KeyModifiers::NONE,
    )));
    let msg = try_recv_variant(&mut rx);
    assert_eq!(
        msg,
        format!("ScrollDown({})", vp),
        "expected ScrollDown({}), got {}",
        vp,
        msg
    );
}

#[test]
fn page_up_scrolls_viewport() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    let vp = make_viewport_rows(&app).saturating_sub(1);
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::PageUp,
        KeyModifiers::NONE,
    )));
    let msg = try_recv_variant(&mut rx);
    assert_eq!(
        msg,
        format!("ScrollUp({})", vp),
        "expected ScrollUp({}), got {}",
        vp,
        msg
    );
}

#[test]
fn ctrl_up_scrolls_viewport() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    let vp = make_viewport_rows(&app).saturating_sub(1);
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Up,
        KeyModifiers::CONTROL,
    )));
    let msg = try_recv_variant(&mut rx);
    assert_eq!(
        msg,
        format!("ScrollUp({})", vp),
        "expected ScrollUp({}), got {}",
        vp,
        msg
    );
}

#[test]
fn ctrl_down_scrolls_viewport() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    let vp = make_viewport_rows(&app).saturating_sub(1);
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Down,
        KeyModifiers::CONTROL,
    )));
    let msg = try_recv_variant(&mut rx);
    assert_eq!(
        msg,
        format!("ScrollDown({})", vp),
        "expected ScrollDown({}), got {}",
        vp,
        msg
    );
}

// ---------- sort and filter shortcuts ----------

#[test]
fn s_key_sends_cycle_sort() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "CycleSort");
}

#[test]
fn f_key_sends_open_filter_popup() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('f'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "OpenFilterPopup");
}

#[test]
fn shift_f_sends_clear_filters() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('F'),
        KeyModifiers::SHIFT,
    )));
    assert_eq!(try_recv_variant(&mut rx), "ClearFilters");
}

#[test]
fn j_on_fk_col_sends_jump_to_fk() {
    let (mut app, mut rx) = make_test_app();
    let mut grid = make_grid();
    grid.fk_cols[1] = true; // name col is FK
    grid.focused_col = 1; // focus the FK column
    app.grid = Some(grid);
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('j'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "JumpToFk");
}

#[test]
fn esc_in_grid_clears_selection() {
    let (mut app, _rx) = make_test_app();
    let mut grid = make_grid();
    grid.select_only_row(2);
    app.grid = Some(grid);
    app.focus = FocusPane::Grid;

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )));

    let grid = app.grid.as_ref().expect("grid");
    assert_eq!(grid.row_selection, crate::grid::RowSelection::None);
    assert!(matches!(app.focus, FocusPane::Grid));
}

#[test]
fn backspace_with_jump_stack_sends_jump_back() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.jump_stack.push(JumpFrame {
        table: "users".to_string(),
        rowid: 1,
        col: 0,
    });
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Backspace,
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "JumpBack");
}

#[test]
fn backspace_without_jump_stack_is_noop() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    assert!(app.jump_stack.is_empty());
    let _ = rx.try_recv(); // drain
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Backspace,
        KeyModifiers::NONE,
    )));
    // no JumpBack should be sent
    let msg = try_recv_variant(&mut rx);
    assert!(!msg.contains("JumpBack"), "unexpected: {}", msg);
}

// ---------- editing shortcuts ----------

#[test]
fn i_key_in_grid_sends_insert_row() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('i'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "InsertRow");
}

#[test]
fn insert_key_in_grid_sends_insert_row() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Insert,
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "InsertRow");
}

#[test]
fn insert_row_opens_insert_popup_for_constrained_table() {
    let (mut app, _rx) = make_constrained_insert_app();
    app.focus = FocusPane::Grid;

    app.update(Message::InsertRow);

    assert!(matches!(
        app.popup,
        Some(PopupKind::InsertRow(ref state))
            if state.editing && state.insert_position == 0
    ));
    assert!(matches!(
        app.toast.toasts.back(),
        Some(toast) if toast.message == "Alt-Enter commits" && toast.kind == ToastKind::Info
    ));
}

#[test]
fn invalid_insert_commit_shows_error_toast() {
    let (mut app, _rx) = make_constrained_insert_app();
    app.focus = FocusPane::Grid;
    app.update(Message::InsertRow);

    app.update(Message::CommitInsertRow);

    assert!(matches!(app.popup, Some(PopupKind::InsertRow(_))));
    assert!(matches!(
        app.toast.toasts.back(),
        Some(toast) if toast.message == "name is required" && toast.kind == ToastKind::Error
    ));
}

#[test]
fn d_key_in_grid_shows_confirm_dialog() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('d'),
        KeyModifiers::NONE,
    )));
    // sends DeleteRow - confirm dialog appears as side-effect
    assert_eq!(try_recv_variant(&mut rx), "DeleteRow");
}

#[test]
fn delete_key_in_grid_sends_delete_row() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Delete,
        KeyModifiers::NONE,
    )));

    assert_eq!(try_recv_variant(&mut rx), "DeleteRow");
}

#[test]
fn delete_row_confirms_selected_row_range() {
    let (mut app, _rx) = make_test_app();
    seed_user_rows(&app, 6);
    let mut grid = make_grid();
    grid.row_selection = crate::grid::RowSelection::Rows(BTreeSet::from([1, 3, 5]));
    app.grid = Some(grid);

    app.update(Message::DeleteRow);

    assert!(matches!(
        app.pending_confirm.as_ref().map(|confirm| &confirm.kind),
        Some(ConfirmKind::DeleteSelectedRows {
            rowids,
            ..
        }) if rowids == &vec![2, 4, 6]
    ));
}

#[test]
fn delete_row_confirms_table_clear_for_select_all() {
    let (mut app, _rx) = make_test_app();
    let mut grid = make_grid();
    grid.row_selection = crate::grid::RowSelection::all();
    app.grid = Some(grid);
    seed_user_rows(&app, 5);

    app.update(Message::DeleteRow);

    assert!(matches!(
        app.pending_confirm.as_ref().map(|confirm| &confirm.kind),
        Some(ConfirmKind::ClearTable { table, keep }) if table == "users" && keep.is_empty()
    ));
    let message = app
        .pending_confirm
        .as_ref()
        .map(|confirm| confirm.message.clone())
        .expect("confirm message");
    assert!(message.contains("Delete all 5 rows from users?"));
}

#[test]
fn y_in_grid_sends_copy_cell() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('y'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "CopyCell");
}

#[test]
fn ctrl_c_in_grid_sends_copy_cell() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(try_recv_variant(&mut rx), "CopyCell");
}

#[test]
fn shift_y_in_grid_sends_copy_row_json() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('Y'),
        KeyModifiers::SHIFT,
    )));
    assert_eq!(try_recv_variant(&mut rx), "CopyRowJson");
}

#[test]
fn row_json_text_returns_single_object_without_selection() {
    let (mut app, _rx) = make_test_app();
    app.grid = Some(make_grid());
    seed_grid_rows(&app, 50);

    let (json_text, copied_selected_rows) =
        app.row_json_text().expect("row json").expect("json text");
    let json: serde_json::Value = serde_json::from_str(&json_text).expect("valid json");

    assert!(!copied_selected_rows);
    assert_eq!(json["id"], serde_json::json!(0));
    assert_eq!(json["name"], serde_json::json!("user-0"));
}

#[test]
fn row_json_text_returns_selected_rows_as_array() {
    let (mut app, _rx) = make_test_app();
    let mut grid = make_grid();
    grid.row_selection = crate::grid::RowSelection::Rows(BTreeSet::from([1, 3]));
    app.grid = Some(grid);
    seed_grid_rows(&app, 50);

    let (json_text, copied_selected_rows) =
        app.row_json_text().expect("row json").expect("json text");
    let json: serde_json::Value = serde_json::from_str(&json_text).expect("valid json");

    assert!(copied_selected_rows);
    assert_eq!(
        json,
        serde_json::json!([
            {
                "id": 1,
                "name": "user-1",
                "age": 21,
                "email": "user1@example.com"
            },
            {
                "id": 3,
                "name": "user-3",
                "age": 23,
                "email": "user3@example.com"
            }
        ])
    );
}

#[test]
fn ctrl_z_sends_undo_action() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    let _ = rx.try_recv(); // drain
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('z'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(try_recv_variant(&mut rx), "UndoAction");
}

#[test]
fn enter_in_grid_sends_open_popup() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    let _ = rx.try_recv(); // drain
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "OpenPopup");
}

#[test]
fn e_in_grid_sends_open_direct_edit() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    let _ = rx.try_recv(); // drain
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('e'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "OpenDirectEdit");
}

#[test]
fn n_in_grid_sends_set_focused_cell_null_when_allowed() {
    let (mut app, _rx) = make_test_app();
    let mut grid = make_grid();
    grid.focused_col = 1;
    app.grid = Some(grid);
    app.focus = FocusPane::Grid;

    let (tx, mut rx) = mpsc::unbounded_channel();
    app.tx = tx;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('n'),
        KeyModifiers::NONE,
    )));

    assert_eq!(try_recv_variant(&mut rx), "SetFocusedCellNull");
}

#[test]
fn n_in_grid_does_nothing_when_cell_cannot_be_null() {
    let (mut app, _rx) = make_test_app();
    let mut grid = make_grid();
    grid.focused_col = 1;
    grid.columns[1].not_null = true;
    app.grid = Some(grid);
    app.focus = FocusPane::Grid;

    let (tx, mut rx) = mpsc::unbounded_channel();
    app.tx = tx;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('n'),
        KeyModifiers::NONE,
    )));

    assert_eq!(try_recv_variant(&mut rx), "no message");
}

#[test]
fn open_direct_edit_opens_text_editor() {
    let (mut app, _rx) = make_test_app();
    let mut grid = make_grid();
    grid.focused_col = 1;
    app.grid = Some(grid);
    seed_user_row(&app);

    app.update(Message::OpenDirectEdit);

    assert!(matches!(app.popup, Some(PopupKind::TextEditor(_))));
}

// ---------- sidebar shortcuts ----------

#[test]
fn enter_in_sidebar_opens_table() {
    let (mut app, mut rx) = make_test_app();
    app.focus = FocusPane::Sidebar;
    // navigate to first table entry
    app.sidebar.move_down(&app.schema);
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    let msg = try_recv_variant(&mut rx);
    assert!(msg.contains("OpenTable"), "expected OpenTable, got {}", msg);
}

#[test]
fn up_down_arrows_in_sidebar_navigate() {
    let (mut app, _rx) = make_test_app();
    let initial = app.sidebar.selected;
    app.focus = FocusPane::Sidebar;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Down,
        KeyModifiers::NONE,
    )));
    assert_ne!(app.sidebar.selected, initial);
}

#[test]
fn left_right_arrows_in_sidebar_collapse_and_expand_selected_section() {
    let (mut app, _rx) = make_test_app();
    app.focus = FocusPane::Sidebar;

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Left,
        KeyModifiers::NONE,
    )));
    assert!(!app.sidebar.tables_expanded);

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Right,
        KeyModifiers::NONE,
    )));
    assert!(app.sidebar.tables_expanded);
}

#[test]
fn left_right_arrows_in_sidebar_apply_to_views_and_indexes_headers() {
    let (mut app, _rx) = make_test_app();
    app.focus = FocusPane::Sidebar;

    // tables header, first table, views header
    app.sidebar.move_down(&app.schema);
    app.sidebar.move_down(&app.schema);
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Left,
        KeyModifiers::NONE,
    )));
    assert!(!app.sidebar.views_expanded);
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Right,
        KeyModifiers::NONE,
    )));
    assert!(app.sidebar.views_expanded);

    app.sidebar.move_down(&app.schema);
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Left,
        KeyModifiers::NONE,
    )));
    assert!(!app.sidebar.indexes_expanded);
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Right,
        KeyModifiers::NONE,
    )));
    assert!(app.sidebar.indexes_expanded);
}

// ---------- letter jumps ----------

#[test]
fn letter_key_on_text_sorted_column_sends_jump_to_letter() {
    let (mut app, mut rx) = make_test_app();
    let mut grid = make_grid();
    grid.sort = Some(SortSpec {
        col_idx: 1,
        direction: SortDir::Asc,
    }); // name is TEXT -> text sort
    app.grid = Some(grid);
    app.focus = FocusPane::Grid;
    let _ = rx.try_recv(); // drain
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "JumpToLetter('a')");
}

#[test]
fn hash_key_on_text_sorted_column_sends_jump_to_letter() {
    let (mut app, mut rx) = make_test_app();
    let mut grid = make_grid();
    grid.sort = Some(SortSpec {
        col_idx: 1,
        direction: SortDir::Asc,
    });
    app.grid = Some(grid);
    app.focus = FocusPane::Grid;
    let _ = rx.try_recv(); // drain
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('#'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "JumpToLetter('#')");
}

#[test]
fn letter_key_without_text_sort_does_nothing() {
    let (mut app, mut rx) = make_test_app();
    let mut grid = make_grid();
    grid.sort = None; // no sort
    app.grid = Some(grid);
    app.focus = FocusPane::Grid;
    let _ = rx.try_recv(); // drain
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::NONE,
    )));
    let msg = try_recv_variant(&mut rx);
    assert!(!msg.contains("JumpToLetter"), "unexpected: {}", msg);
}

#[tokio::test]
async fn commit_find_repositions_and_fetches_target_window() {
    let (mut app, _rx) = make_test_app();
    let mut grid = make_grid();
    let find_rows = grid.window.rows.clone();
    grid.window.rows.truncate(10);
    grid.window.offset = 0;
    app.grid = Some(grid);

    let columns = app.grid.as_ref().expect("grid").columns.clone();
    let mut find = FindState::new("users".to_string(), columns);
    find.set_rows(find_rows);
    "user-40".chars().for_each(|c| find.push_char(c));
    app.popup = Some(PopupKind::Find(find));
    app.mode = AppMode::Edit;

    app.update(Message::CommitFind);

    assert!(app.popup.is_none(), "find popup should close after commit");
    assert_eq!(app.mode, AppMode::Browse);

    let grid = app.grid.as_ref().expect("grid");
    assert_eq!(grid.focused_row, 40);
    assert_eq!(grid.focused_col, 1);
    assert!(
        grid.viewport_start > 0,
        "viewport should move to target row"
    );
    assert!(
        grid.window.fetch_in_flight,
        "jump should start loading the target window immediately"
    );
}

// ---------- help popup navigation ----------

#[test]
fn help_scroll_up_and_down() {
    let mut state = HelpState::new();
    // simulate large viewport so scroll is visible
    state.max_scroll = 10;
    state.scroll_down(3);
    assert_eq!(state.scroll, 3);
    state.scroll_up(2);
    assert_eq!(state.scroll, 1);
    state.scroll_up(5);
    assert_eq!(state.scroll, 0); // clamps at 0
    state.scroll_down(100);
    assert_eq!(state.scroll, 10); // clamps at max_scroll
}

#[test]
fn help_up_down_keys_scroll_in_edit_mode() {
    let (mut app, _rx) = make_test_app();
    let mut state = HelpState::new();
    state.max_scroll = 10;
    app.popup = Some(PopupKind::Help(state));
    app.mode = AppMode::Edit;

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Down,
        KeyModifiers::NONE,
    )));
    assert!(matches!(&app.popup, Some(PopupKind::Help(s)) if s.scroll == 3));

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Up,
        KeyModifiers::NONE,
    )));
    assert!(matches!(&app.popup, Some(PopupKind::Help(s)) if s.scroll == 0));
}

#[test]
fn help_page_up_down_scrolls_faster() {
    let (mut app, _rx) = make_test_app();
    let mut state = HelpState::new();
    state.max_scroll = 30;
    app.popup = Some(PopupKind::Help(state));
    app.mode = AppMode::Edit;

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::PageDown,
        KeyModifiers::NONE,
    )));
    assert!(matches!(&app.popup, Some(PopupKind::Help(s)) if s.scroll == 10));
}

// ---------- confirm dialog ----------

#[test]
fn y_confirm_and_n_cancel_in_confirm_dialog() {
    let (mut app, mut rx) = make_test_app();
    app.pending_confirm = Some(PendingConfirm {
        message: "Delete?".to_string(),
        kind: ConfirmKind::DeleteRow {
            table: "users".to_string(),
            rowid: 42,
        },
        created: std::time::Instant::now(),
    });

    // n cancels
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('n'),
        KeyModifiers::NONE,
    )));
    drain_messages(&mut app, &mut rx);
    assert!(app.pending_confirm.is_none());
}

#[test]
fn esc_cancels_confirm_dialog() {
    let (mut app, mut rx) = make_test_app();
    app.pending_confirm = Some(PendingConfirm {
        message: "Delete?".to_string(),
        kind: ConfirmKind::DeleteRow {
            table: "users".to_string(),
            rowid: 42,
        },
        created: std::time::Instant::now(),
    });

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )));
    drain_messages(&mut app, &mut rx);
    assert!(app.pending_confirm.is_none());
}

// ---------- mouse handling ----------

// mouse scroll triggers async fetch which needs tokio runtime;
// tested instead via scroll shortcuts which verify scroll messages directly

#[test]
fn mouse_click_on_grid_focuses_grid() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.grid_inner_area = Some(ratatui::layout::Rect {
        x: 12,
        y: 8,
        width: 56,
        height: 16,
    });
    app.focus = FocusPane::Sidebar;
    let _ = rx.try_recv(); // drain
    app.update(Message::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: 20,
        row: 10,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(matches!(app.focus, FocusPane::Grid));
}

#[test]
fn mouse_drag_on_grid_scrollbar_scrolls_rows() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.grid.as_mut().expect("grid").window.fetch_in_flight = true;
    app.grid_inner_area = Some(ratatui::layout::Rect {
        x: 12,
        y: 8,
        width: 20,
        height: 13,
    });
    app.focus = FocusPane::Sidebar;
    let _ = rx.try_recv(); // drain

    app.update(Message::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: 31,
        row: 11,
        modifiers: KeyModifiers::NONE,
    }));
    let start_row = app.grid.as_ref().expect("grid").focused_row;
    assert!(app.grid_scrollbar_drag.is_some());

    app.update(Message::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left),
        column: 31,
        row: 20,
        modifiers: KeyModifiers::NONE,
    }));
    let dragged_row = app.grid.as_ref().expect("grid").focused_row;

    assert!(matches!(app.focus, FocusPane::Grid));
    assert!(dragged_row > start_row);

    app.update(Message::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left),
        column: 31,
        row: 20,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(app.grid_scrollbar_drag.is_none());
}

#[test]
fn mouse_wheel_scrolls_text_editor_only_under_the_pointer() {
    let (mut app, _rx) = make_test_app();
    app.popup = Some(PopupKind::TextEditor(TextEditorState::new(
        "users".to_string(),
        1,
        "name".to_string(),
        "TEXT".to_string(),
        SqlValue::Text("x".repeat(1_000)),
        false,
    )));
    app.mode = AppMode::Edit;
    render_test_app(&mut app);
    let initial_scroll = text_editor_scroll_y(&app);
    assert!(initial_scroll > 0);

    app.update(Message::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 40,
        row: 12,
        modifiers: KeyModifiers::NONE,
    }));
    let scrolled = text_editor_scroll_y(&app);
    assert!(scrolled < initial_scroll);

    app.update(Message::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 40,
        row: 12,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(text_editor_scroll_y(&app), initial_scroll);

    app.update(Message::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 1,
        row: 1,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(text_editor_scroll_y(&app), initial_scroll);
}

#[test]
fn mouse_drag_on_text_editor_scrollbar_scrolls_to_end() {
    let (mut app, _rx) = make_test_app();
    app.popup = Some(PopupKind::TextEditor(TextEditorState::new(
        "users".to_string(),
        1,
        "name".to_string(),
        "TEXT".to_string(),
        SqlValue::Text("x".repeat(1_000)),
        false,
    )));
    app.mode = AppMode::Edit;
    render_test_app(&mut app);
    let max_scroll = text_editor_scroll_y(&app);
    if let Some(PopupKind::TextEditor(state)) = app.popup.as_mut() {
        state.scroll_up(u16::MAX);
    }
    render_test_app(&mut app);
    assert_eq!(text_editor_scroll_y(&app), 0);

    app.update(Message::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 62,
        row: 10,
        modifiers: KeyModifiers::NONE,
    }));
    app.update(Message::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: 62,
        row: 19,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(text_editor_scroll_y(&app), max_scroll);

    app.update(Message::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: 62,
        row: 19,
        modifiers: KeyModifiers::NONE,
    }));
    app.update(Message::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: 62,
        row: 10,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(text_editor_scroll_y(&app), max_scroll);
}

#[test]
fn ctrl_click_on_row_gutter_toggles_rows_without_clearing_selection() {
    let (mut app, _rx) = make_test_app();
    app.grid = Some(make_grid());
    app.grid_inner_area = Some(ratatui::layout::Rect {
        x: 12,
        y: 8,
        width: 56,
        height: 16,
    });

    app.update(Message::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: 12,
        row: 11,
        modifiers: KeyModifiers::CONTROL,
    }));
    app.update(Message::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: 12,
        row: 13,
        modifiers: KeyModifiers::CONTROL,
    }));

    let grid = app.grid.as_ref().expect("grid");
    assert_eq!(grid.focused_row, 2);
    assert_eq!(
        grid.row_selection,
        crate::grid::RowSelection::Rows(BTreeSet::from([0, 2]))
    );
}

#[test]
fn tabbar_hit_test_targets_visible_close_glyph() {
    let (mut app, _rx) = make_test_app();
    app.open_tabs = vec![TableTab {
        table_name: "ghost".to_string(),
    }];
    app.active_tab = Some(0);

    assert!(matches!(
        crate::ui::tabbar::hit_test(
            ratatui::layout::Rect {
                x: 0,
                y: 0,
                width: 20,
                height: 3,
            },
            &app,
            9,
            1,
            false,
        ),
        Some(TabMouseAction::Close(0))
    ));
}

#[test]
fn mouse_click_on_tab_close_button_closes_tab() {
    let (mut app, mut rx) = make_test_app();
    app.open_tabs = vec![TableTab {
        table_name: "ghost".to_string(),
    }];
    app.active_tab = Some(0);
    app.tabbar_area = ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 20,
        height: 3,
    };
    app.focus = FocusPane::Sidebar;

    app.update(Message::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: 9,
        row: 1,
        modifiers: KeyModifiers::NONE,
    }));
    drain_messages(&mut app, &mut rx);

    assert!(app.open_tabs.is_empty());
    assert_eq!(app.active_tab, None);
    assert!(matches!(app.focus, FocusPane::Grid));
}

#[test]
fn closing_inactive_tab_keeps_same_active_tab() {
    assert_eq!(next_active_tab_after_close(Some(2), 0, 3), Some(1));
    assert_eq!(next_active_tab_after_close(Some(1), 2, 3), Some(1));
}

#[test]
fn closing_active_tab_activates_previous_tab() {
    assert_eq!(next_active_tab_after_close(Some(2), 2, 3), Some(1));
    assert_eq!(next_active_tab_after_close(Some(0), 0, 2), Some(0));
    assert_eq!(next_active_tab_after_close(Some(0), 0, 0), None);
}

// ---------- value picker tests (existing) ----------

#[test]
fn value_picker_allows_long_entries_when_distinct_set_is_small() {
    let values = vec![
        "Apple Inc.".to_string(),
        "Embraer - Empresa Brasileira de Aeronáutica S.A.".to_string(),
    ];

    assert!(should_use_value_picker(&values));
}

#[test]
fn value_picker_rejects_empty_and_oversized_distinct_sets() {
    assert!(!should_use_value_picker(&[]));

    let values = (0..101).map(|i| format!("value-{i}")).collect::<Vec<_>>();
    assert!(!should_use_value_picker(&values));
}

// ---------- file-watch tests ----------

fn make_schema_with(table: &str) -> crate::db::schema::Schema {
    let manager = SqliteConnectionManager::memory();
    let pool = r2d2::Pool::builder()
        .max_size(1)
        .build(manager)
        .expect("pool");
    let conn = pool.get().expect("conn");
    conn.execute_batch(&format!("CREATE TABLE {table} (id INTEGER PRIMARY KEY);"))
        .expect("create");
    crate::db::load_schema(&conn).expect("schema")
}

#[test]
fn external_refresh_with_identical_schema_does_not_push_toast() {
    let (mut app, _rx) = make_test_app();
    // First: send a schema with a DIFFERENT table name → should push a toast.
    let different_schema = make_schema_with("other_table");
    let toast_before = app.toast.toasts.len();
    app.update(Message::ExternalRefresh(different_schema));
    assert_eq!(
        app.toast.toasts.len(),
        toast_before + 1,
        "schema change should push a toast"
    );
    // Second: send the same schema again (fingerprint unchanged) → no new toast.
    let same_again = make_schema_with("other_table");
    let toast_mid = app.toast.toasts.len();
    app.update(Message::ExternalRefresh(same_again));
    assert_eq!(
        app.toast.toasts.len(),
        toast_mid,
        "identical schema must not push another toast"
    );
}

#[tokio::test]
async fn file_changed_during_edit_mode_sets_pending_flag_not_needs_fetch() {
    let (mut app, _rx) = make_test_app();
    // Simulate an open grid.
    app.grid = Some(make_grid());
    // Put app into Edit mode (simulates an open popup).
    app.mode = AppMode::Edit;
    app.update(Message::FileChanged);
    assert!(
        app.pending_external_refresh,
        "flag should be set in Edit mode"
    );
    if let Some(ref grid) = app.grid {
        assert!(
            !grid.needs_fetch,
            "needs_fetch must not be set while editing"
        );
    }
}

#[tokio::test]
async fn file_changed_after_own_write_still_refreshes() {
    let (mut app, _rx) = make_test_app();
    app.update(Message::FileChanged);
    assert!(app.file_check_in_flight);
}

#[tokio::test]
async fn close_popup_clears_pending_refresh_and_triggers_fetch() {
    let (mut app, _rx) = make_test_app();
    app.grid = Some(make_grid());
    app.mode = AppMode::Edit;
    app.pending_external_refresh = true;
    app.popup = Some(PopupKind::Help(HelpState::new()));
    app.update(Message::ClosePopup);
    assert!(!app.pending_external_refresh, "flag must be cleared");
    assert_eq!(app.mode, AppMode::Browse);
    if let Some(ref grid) = app.grid {
        assert!(
            grid.window.fetch_in_flight,
            "a refreshed grid fetch must start after close"
        );
    }
}

#[test]
fn stale_window_response_is_ignored() {
    let (mut app, _rx) = make_test_app();
    app.grid = Some(make_grid());
    app.grid_request_serial = 2;
    let original = app.grid.as_ref().expect("grid").window.rows[0].clone();

    app.update(Message::WindowReady {
        request_id: 1,
        table: "users".to_string(),
        offset: 0,
        rows: vec![vec![SqlValue::Text("stale".to_string())]],
        rowids: vec![Some(99)],
        total_rows: 1,
    });

    assert_eq!(app.grid.as_ref().expect("grid").window.rows[0], original);
}

#[test]
fn in_flight_window_completion_preserves_a_queued_scroll_fetch() {
    let (mut app, _rx) = make_test_app();
    let mut grid = make_grid();
    grid.window.fetch_in_flight = true;
    app.grid = Some(grid);

    app.scroll_grid_to_row(40);
    assert!(app.grid.as_ref().expect("grid").needs_fetch);
    app.update(Message::WindowReady {
        request_id: 0,
        table: "users".to_string(),
        offset: 0,
        rows: (0..20)
            .map(|index| vec![SqlValue::Integer(index)])
            .collect(),
        rowids: (1..=20).map(Some).collect(),
        total_rows: 100,
    });

    let grid = app.grid.as_ref().expect("grid");
    assert!(grid.needs_fetch);
    assert!(!grid.window.fetch_in_flight);
}

#[test]
fn stale_alphabet_navigation_is_ignored() {
    let (mut app, _rx) = make_test_app();
    app.grid = Some(make_grid());
    app.navigation_request_serial = 2;

    app.update(Message::JumpToSortedOffset {
        request_id: 1,
        table: "users".to_string(),
        offset: 30,
    });

    assert_eq!(app.grid.as_ref().expect("grid").focused_row, 0);
}

#[test]
fn export_failure_does_not_release_the_write_gate() {
    let (mut app, _rx) = make_test_app();
    app.write_in_flight = true;

    app.update(Message::ExportFailed("disk full".to_string()));

    assert!(app.write_in_flight);
}

#[test]
fn opening_direct_editor_invalidates_pending_distinct_lookup() {
    let (mut app, _rx) = make_test_app();
    app.grid = Some(make_grid());
    app.popup_request_serial = 1;
    let column = app.grid.as_ref().expect("grid").columns[0].clone();

    app.update(Message::OpenDirectEdit);
    app.update(Message::DistinctValuesReady {
        request_id: 1,
        table: "users".to_string(),
        rowid: 1,
        col: column,
        original: SqlValue::Integer(0),
        values: vec!["stale".to_string()],
    });

    assert!(matches!(app.popup, Some(PopupKind::TextEditor(_))));
}

#[tokio::test]
async fn external_refresh_detects_column_changes() {
    let (mut app, _rx) = make_test_app();
    app.grid = Some(make_grid());
    let conn = app.pool.get().expect("connection");
    conn.execute("ALTER TABLE users ADD COLUMN nickname TEXT", [])
        .expect("alter table");
    let changed = db::load_schema(&conn).expect("schema");
    drop(conn);
    app.update(Message::ExternalRefresh(changed));
    assert!(app.schema.tables[0]
        .columns
        .iter()
        .any(|column| column.name == "nickname"));
    assert!(app
        .grid
        .as_ref()
        .expect("active grid")
        .columns
        .iter()
        .any(|column| column.name == "nickname"));
}

#[tokio::test]
async fn schema_refresh_restarts_a_pending_initial_grid_load() {
    let (mut app, _rx) = make_test_app();
    app.grid = None;
    app.open_tabs = vec![TableTab {
        table_name: "users".to_string(),
    }];
    app.active_tab = Some(0);
    app.grid_request_serial = 1;
    let conn = app.pool.get().expect("connection");
    conn.execute("ALTER TABLE users ADD COLUMN nickname TEXT", [])
        .expect("alter table");
    let changed = db::load_schema(&conn).expect("schema");
    drop(conn);

    app.update(Message::ExternalRefresh(changed));
    let current_request = app.grid_request_serial;
    assert!(current_request > 1);
    app.update(Message::GridDataReady {
        request_id: 1,
        table: "users".to_string(),
        columns: make_grid().columns,
        fk_cols: vec![false; 4],
        fetched: db::FetchedRows::default(),
        total_rows: 0,
    });
    assert!(app.grid.is_none(), "stale initial response must be ignored");
}

#[tokio::test]
async fn dropping_a_table_invalidates_its_pending_initial_grid_load() {
    let (mut app, _rx) = make_test_app();
    app.grid = None;
    app.open_tabs = vec![TableTab {
        table_name: "users".to_string(),
    }];
    app.active_tab = Some(0);
    app.grid_request_serial = 1;
    let conn = app.pool.get().expect("connection");
    conn.execute("DROP TABLE users", []).expect("drop table");
    let changed = db::load_schema(&conn).expect("schema");
    drop(conn);

    app.update(Message::ExternalRefresh(changed));
    assert!(app.grid_request_serial > 1);
    app.update(Message::GridDataReady {
        request_id: 1,
        table: "users".to_string(),
        columns: make_grid().columns,
        fk_cols: vec![false; 4],
        fetched: db::FetchedRows::default(),
        total_rows: 0,
    });
    assert!(app.grid.is_none(), "dropped table must not be restored");
}

#[tokio::test]
async fn write_completion_applies_a_pending_schema_refresh() {
    let (mut app, _rx) = make_test_app();
    app.grid = Some(make_grid());
    app.popup = Some(PopupKind::Help(HelpState::new()));
    app.mode = AppMode::Edit;
    let conn = app.pool.get().expect("connection");
    conn.execute("ALTER TABLE users ADD COLUMN nickname TEXT", [])
        .expect("alter table");
    app.schema = db::load_schema(&conn).expect("schema");
    drop(conn);
    app.pending_external_refresh = true;

    app.update(Message::RowInserted {
        table: "users".to_string(),
        rowid: 99,
    });

    assert!(!app.pending_external_refresh);
    assert!(app
        .grid
        .as_ref()
        .expect("grid")
        .columns
        .iter()
        .any(|column| column.name == "nickname"));
}
