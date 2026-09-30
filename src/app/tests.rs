use std::{collections::BTreeSet, sync::Arc};

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use r2d2_sqlite::SqliteConnectionManager;
use ratatui::{backend::TestBackend, Terminal};
use tokio::sync::mpsc;

use super::*;
use crate::{
    config::Config,
    db::{self, schema::Column, types::SqlValue},
    ui::{
        popup::{command_palette::CopyFormat, HelpState},
        tabbar::TabMouseAction,
    },
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

/// Waits for the next message from background work and applies it.
async fn receive_one(app: &mut App, rx: &mut mpsc::UnboundedReceiver<Message>) {
    let message = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("background work should reply")
        .expect("channel open");
    app.update(message);
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

#[tokio::test]
async fn ctrl_q_sets_should_quit() {
    let (mut app, _rx) = make_test_app();
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('q'),
        KeyModifiers::CONTROL,
    )));
    assert!(app.should_quit);
}

#[tokio::test]
async fn ctrl_b_toggles_sidebar() {
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

#[tokio::test]
async fn ctrl_w_closes_current_tab() {
    let (mut app, mut rx) = make_test_app();
    app.open_tabs = vec![TableTab::new("users".to_string())];
    app.active_tab = Some(0);

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('w'),
        KeyModifiers::CONTROL,
    )));
    drain_messages(&mut app, &mut rx);

    assert!(app.open_tabs.is_empty());
    assert_eq!(app.active_tab, None);
}

#[tokio::test]
async fn tab_toggles_focus_sidebar_to_grid() {
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

#[tokio::test]
async fn backtab_toggles_focus() {
    let (mut app, _rx) = make_test_app();
    app.sidebar_visible = true;
    app.focus = FocusPane::Sidebar;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::BackTab,
        KeyModifiers::NONE,
    )));
    assert!(matches!(app.focus, FocusPane::Grid));
}

#[tokio::test]
async fn question_mark_opens_and_closes_help() {
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

#[tokio::test]
async fn ctrl_p_opens_command_palette() {
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

#[tokio::test]
async fn esc_in_help_closes_popup() {
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

#[tokio::test]
async fn ctrl_enter_in_help_is_noop() {
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

#[tokio::test]
async fn ctrl_enter_in_text_editor_is_noop() {
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

#[tokio::test]
async fn enter_in_text_editor_sends_commit_edit() {
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

#[tokio::test]
async fn alt_enter_in_text_editor_inserts_newline() {
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

#[tokio::test]
async fn alt_enter_in_insert_row_sends_commit_insert_row() {
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

#[tokio::test]
async fn arrow_down_sends_move_down() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Down,
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveDown");
}

#[tokio::test]
async fn arrow_up_sends_move_up() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Up,
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveUp");
}

#[tokio::test]
async fn shift_down_selects_rows_from_focused_row() {
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

#[tokio::test]
async fn shift_up_extends_selection_toward_previous_rows() {
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

#[tokio::test]
async fn move_down_preserves_existing_selection() {
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

#[tokio::test]
async fn ctrl_a_selects_all_rows_in_grid() {
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

#[tokio::test]
async fn arrow_left_sends_move_left() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Left,
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveLeft");
}

#[tokio::test]
async fn arrow_right_sends_move_right() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Right,
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveRight");
}

#[tokio::test]
async fn vim_hjkl_navigation() {
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

#[tokio::test]
async fn home_sends_move_col_first() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Home,
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveColFirst");
}

#[tokio::test]
async fn end_sends_move_col_last() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::End,
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveColLast");
}

#[tokio::test]
async fn ctrl_home_sends_move_first_cell() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Home,
        KeyModifiers::CONTROL,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveFirstCell");
}

#[tokio::test]
async fn ctrl_end_sends_move_last_cell() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::End,
        KeyModifiers::CONTROL,
    )));
    assert_eq!(try_recv_variant(&mut rx), "MoveLastCell");
}

#[tokio::test]
async fn page_down_scrolls_viewport() {
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

#[tokio::test]
async fn page_up_scrolls_viewport() {
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

#[tokio::test]
async fn ctrl_up_scrolls_viewport() {
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

#[tokio::test]
async fn ctrl_down_scrolls_viewport() {
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

#[tokio::test]
async fn s_key_sends_cycle_sort() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "CycleSort");
}

#[tokio::test]
async fn f_key_sends_open_filter_popup() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('f'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "OpenFilterPopup");
}

#[tokio::test]
async fn shift_f_sends_clear_filters() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('F'),
        KeyModifiers::SHIFT,
    )));
    assert_eq!(try_recv_variant(&mut rx), "ClearFilters");
}

#[tokio::test]
async fn j_on_fk_col_sends_jump_to_fk() {
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

#[tokio::test]
async fn esc_in_grid_clears_selection() {
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

#[tokio::test]
async fn backspace_with_jump_stack_sends_jump_back() {
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

#[tokio::test]
async fn backspace_without_jump_stack_is_noop() {
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

#[tokio::test]
async fn i_key_in_grid_sends_insert_row() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('i'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "InsertRow");
}

#[tokio::test]
async fn insert_key_in_grid_sends_insert_row() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Insert,
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "InsertRow");
}

#[tokio::test]
async fn insert_row_opens_insert_popup_for_constrained_table() {
    let (mut app, _rx) = make_constrained_insert_app();
    app.focus = FocusPane::Grid;

    app.update(Message::InsertRow);

    assert!(matches!(
        app.popup,
        Some(PopupKind::InsertRow(ref state))
            if state.insert_position == 0
    ));
}

#[tokio::test]
async fn invalid_insert_commit_shows_error_toast() {
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

#[tokio::test]
async fn d_key_in_grid_shows_confirm_dialog() {
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

#[tokio::test]
async fn delete_key_in_grid_sends_delete_row() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Delete,
        KeyModifiers::NONE,
    )));

    assert_eq!(try_recv_variant(&mut rx), "DeleteRow");
}

#[tokio::test]
async fn delete_row_confirms_selected_row_range() {
    let (mut app, mut rx) = make_test_app();
    seed_user_rows(&app, 6);
    let mut grid = make_grid();
    grid.row_selection = crate::grid::RowSelection::Rows(BTreeSet::from([1, 3, 5]));
    app.grid = Some(grid);

    app.update(Message::DeleteRow);
    receive_one(&mut app, &mut rx).await;

    assert!(matches!(
        app.pending_confirm.as_ref().map(|confirm| &confirm.kind),
        Some(ConfirmKind::DeleteSelectedRows {
            rowids,
            ..
        }) if rowids == &vec![2, 4, 6]
    ));
}

#[tokio::test]
async fn delete_row_confirms_table_clear_for_select_all() {
    let (mut app, mut rx) = make_test_app();
    let mut grid = make_grid();
    grid.row_selection = crate::grid::RowSelection::all();
    app.grid = Some(grid);
    seed_user_rows(&app, 5);

    app.update(Message::DeleteRow);
    receive_one(&mut app, &mut rx).await;

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

#[tokio::test]
async fn y_in_grid_sends_copy_cell() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('y'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "CopyCell");
}

#[tokio::test]
async fn ctrl_c_in_grid_sends_copy_cell() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(try_recv_variant(&mut rx), "CopyCell");
}

#[tokio::test]
async fn shift_y_in_grid_sends_copy_row_json() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('Y'),
        KeyModifiers::SHIFT,
    )));
    assert_eq!(try_recv_variant(&mut rx), "CopyRows(Json)");
}

#[tokio::test]
async fn ctrl_z_sends_undo_action() {
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

#[tokio::test]
async fn enter_in_grid_sends_open_popup() {
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

#[tokio::test]
async fn e_in_grid_sends_open_direct_edit() {
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

#[tokio::test]
async fn n_in_grid_sends_set_focused_cell_null_when_allowed() {
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

#[tokio::test]
async fn n_in_grid_does_nothing_when_cell_cannot_be_null() {
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

#[tokio::test]
async fn open_direct_edit_opens_text_editor() {
    let (mut app, _rx) = make_test_app();
    let mut grid = make_grid();
    grid.focused_col = 1;
    app.grid = Some(grid);
    seed_user_row(&app);

    app.update(Message::OpenDirectEdit);

    assert!(matches!(app.popup, Some(PopupKind::TextEditor(_))));
}

// ---------- sidebar shortcuts ----------

#[tokio::test]
async fn enter_in_sidebar_opens_table() {
    let (mut app, mut rx) = make_test_app();
    app.focus = FocusPane::Sidebar;
    // navigate to first table entry
    app.sidebar.move_down(&app.schema);
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    drain_messages(&mut app, &mut rx);
    assert_eq!(app.active_table_name().as_deref(), Some("users"));
    assert!(matches!(app.focus, FocusPane::Grid));
}

#[tokio::test]
async fn up_down_arrows_in_sidebar_navigate() {
    let (mut app, _rx) = make_test_app();
    let initial = app.sidebar.selected;
    app.focus = FocusPane::Sidebar;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Down,
        KeyModifiers::NONE,
    )));
    assert_ne!(app.sidebar.selected, initial);
}

#[tokio::test]
async fn left_right_arrows_in_sidebar_collapse_and_expand_selected_section() {
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

#[tokio::test]
async fn left_right_arrows_in_sidebar_apply_to_views_and_indexes_headers() {
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

#[tokio::test]
async fn letter_key_on_text_sorted_column_sends_jump_to_letter() {
    let (mut app, mut rx) = make_test_app();
    let mut grid = make_grid();
    grid.cycle_sort(1); // name is TEXT -> text sort
    app.grid = Some(grid);
    app.focus = FocusPane::Grid;
    let _ = rx.try_recv(); // drain
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('\''),
        KeyModifiers::NONE,
    )));
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "JumpToLetter('a')");
}

#[tokio::test]
async fn hash_key_on_text_sorted_column_sends_jump_to_letter() {
    let (mut app, mut rx) = make_test_app();
    let mut grid = make_grid();
    grid.cycle_sort(1);
    app.grid = Some(grid);
    app.focus = FocusPane::Grid;
    let _ = rx.try_recv(); // drain
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('\''),
        KeyModifiers::NONE,
    )));
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('#'),
        KeyModifiers::NONE,
    )));
    assert_eq!(try_recv_variant(&mut rx), "JumpToLetter('#')");
}

#[tokio::test]
async fn letter_key_without_text_sort_does_nothing() {
    let (mut app, mut rx) = make_test_app();
    let grid = make_grid();
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

// ---------- help popup navigation ----------

#[tokio::test]
async fn question_mark_types_into_text_popups() {
    let (mut app, _rx) = make_test_app();
    app.grid = Some(make_grid());
    app.update(Message::OpenDirectEdit);
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('?'),
        KeyModifiers::NONE,
    )));
    assert!(
        matches!(&app.popup, Some(PopupKind::TextEditor(state)) if state.current.ends_with('?'))
    );
}

#[tokio::test]
async fn help_stacks_over_a_popup_and_restores_it() {
    let (mut app, mut rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.open_record();
    assert!(matches!(app.popup, Some(PopupKind::Record(_))));
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('?'),
        KeyModifiers::NONE,
    )));
    drain_messages(&mut app, &mut rx);
    assert!(matches!(app.popup, Some(PopupKind::Help(_))));
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )));
    drain_messages(&mut app, &mut rx);
    assert!(
        matches!(app.popup, Some(PopupKind::Record(_))),
        "closing help restores the record"
    );
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )));
    drain_messages(&mut app, &mut rx);
    assert!(app.popup.is_none());
    assert_eq!(app.mode, AppMode::Browse);
}

// ---------- confirm dialog ----------

#[tokio::test]
async fn y_confirm_and_n_cancel_in_confirm_dialog() {
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

#[tokio::test]
async fn esc_cancels_confirm_dialog() {
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

#[tokio::test]
async fn mouse_click_on_grid_focuses_grid() {
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

#[tokio::test]
async fn mouse_drag_on_grid_scrollbar_scrolls_rows() {
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

#[tokio::test]
async fn mouse_wheel_scrolls_text_editor_only_under_the_pointer() {
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

#[tokio::test]
async fn mouse_drag_on_text_editor_scrollbar_scrolls_to_end() {
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

#[tokio::test]
async fn ctrl_click_on_row_gutter_toggles_rows_without_clearing_selection() {
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

#[tokio::test]
async fn tabbar_hit_test_targets_visible_close_glyph() {
    let (mut app, _rx) = make_test_app();
    app.open_tabs = vec![TableTab::new("ghost".to_string())];
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

#[tokio::test]
async fn mouse_click_on_tab_close_button_closes_tab() {
    let (mut app, mut rx) = make_test_app();
    app.open_tabs = vec![TableTab::new("ghost".to_string())];
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
    assert!(
        matches!(app.focus, FocusPane::Sidebar),
        "closing the last tab returns to the sidebar"
    );
}

#[tokio::test]
async fn closing_inactive_tab_keeps_same_active_tab() {
    assert_eq!(next_active_tab_after_close(Some(2), 0, 3), Some(1));
    assert_eq!(next_active_tab_after_close(Some(1), 2, 3), Some(1));
}

#[tokio::test]
async fn closing_active_tab_activates_previous_tab() {
    assert_eq!(next_active_tab_after_close(Some(2), 2, 3), Some(1));
    assert_eq!(next_active_tab_after_close(Some(0), 0, 2), Some(0));
    assert_eq!(next_active_tab_after_close(Some(0), 0, 0), None);
}

// ---------- value picker tests (existing) ----------

#[tokio::test]
async fn value_picker_allows_long_entries_when_distinct_set_is_small() {
    let values = vec![
        "Apple Inc.".to_string(),
        "Embraer - Empresa Brasileira de Aeronáutica S.A.".to_string(),
    ];

    assert!(should_use_value_picker(&values));
}

#[tokio::test]
async fn value_picker_rejects_empty_and_oversized_distinct_sets() {
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

#[tokio::test]
async fn external_refresh_with_identical_schema_does_not_push_toast() {
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

#[tokio::test]
async fn stale_window_response_is_ignored() {
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
        total_rows: Some(1),
    });

    assert_eq!(app.grid.as_ref().expect("grid").window.rows[0], original);
}

#[tokio::test]
async fn in_flight_window_completion_preserves_a_queued_scroll_fetch() {
    let (mut app, _rx) = make_test_app();
    let mut grid = make_grid();
    grid.window.fetch_in_flight = true;
    grid.window.total_rows = 100;
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
        total_rows: Some(100),
    });

    let grid = app.grid.as_ref().expect("grid");
    assert!(grid.needs_fetch);
    assert!(!grid.window.fetch_in_flight);
}

#[tokio::test]
async fn stale_alphabet_navigation_is_ignored() {
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

#[tokio::test]
async fn export_failure_does_not_release_the_write_gate() {
    let (mut app, _rx) = make_test_app();
    app.write_in_flight = true;

    app.update(Message::ExportFailed("disk full".to_string()));

    assert!(app.write_in_flight);
}

#[tokio::test]
async fn opening_direct_editor_invalidates_pending_distinct_lookup() {
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
    app.open_tabs = vec![TableTab::new("users".to_string())];
    app.active_tab = Some(0);
    app.grid_request_serial = 1;
    let conn = app.pool.get().expect("connection");
    conn.execute("ALTER TABLE users ADD COLUMN nickname TEXT", [])
        .expect("alter table");
    let changed = db::load_schema(&conn).expect("schema");
    drop(conn);

    app.update(Message::ExternalRefresh(changed));
    assert!(app.grid_request_serial > 1);
    app.update(Message::WindowReady {
        request_id: 1,
        table: "users".to_string(),
        offset: 0,
        rows: vec![vec![SqlValue::Integer(7)]],
        rowids: vec![Some(7)],
        total_rows: Some(1),
    });
    let grid = app
        .grid
        .as_ref()
        .expect("the grid is rebuilt from the new schema");
    assert!(grid.columns.iter().any(|column| column.name == "nickname"));
    assert!(
        grid.window.rows.is_empty(),
        "stale initial response must be ignored"
    );
}

#[tokio::test]
async fn dropping_a_table_invalidates_its_pending_initial_grid_load() {
    let (mut app, _rx) = make_test_app();
    app.grid = None;
    app.open_tabs = vec![TableTab::new("users".to_string())];
    app.active_tab = Some(0);
    app.grid_request_serial = 1;
    let conn = app.pool.get().expect("connection");
    conn.execute("DROP TABLE users", []).expect("drop table");
    let changed = db::load_schema(&conn).expect("schema");
    drop(conn);

    app.update(Message::ExternalRefresh(changed));
    assert!(app.grid_request_serial > 1);
    app.update(Message::WindowReady {
        request_id: 1,
        table: "users".to_string(),
        offset: 0,
        rows: Vec::new(),
        rowids: Vec::new(),
        total_rows: Some(0),
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

// ---------- audit features ----------

fn make_related_app(readonly: bool) -> (App, mpsc::UnboundedReceiver<Message>) {
    let manager = SqliteConnectionManager::memory();
    let pool = Arc::new(
        r2d2::Pool::builder()
            .max_size(1)
            .build(manager)
            .expect("test pool"),
    );
    let conn = pool.get().expect("test conn");
    conn.execute_batch(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);
         CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER REFERENCES users(id), item TEXT);
         CREATE VIEW big_orders AS SELECT * FROM orders WHERE id > 1;
         INSERT INTO users VALUES (1, 'Ada'), (2, 'Bob');
         INSERT INTO orders VALUES (1, 1, 'pen'), (2, 1, 'ink'), (3, 2, 'pad');",
    )
    .expect("seed schema");
    let schema = db::load_schema(&conn).expect("load schema");
    drop(conn);
    let (tx, rx) = mpsc::unbounded_channel();
    let app = App::new(
        schema,
        Config::default(),
        pool,
        tx,
        readonly,
        ":memory:".to_string(),
    );
    (app, rx)
}

/// Applies background replies until the active grid has its rows.
async fn settle(app: &mut App, rx: &mut mpsc::UnboundedReceiver<Message>) {
    for _ in 0..20 {
        let loaded = app
            .grid
            .as_ref()
            .is_some_and(|grid| grid.count_known && !grid.window.fetch_in_flight);
        if loaded {
            while let Ok(message) = rx.try_recv() {
                app.update(message);
            }
            return;
        }
        receive_one(app, rx).await;
    }
    panic!("grid did not load");
}

fn key(code: KeyCode) -> Message {
    Message::Key(crossterm::event::KeyEvent::new(code, KeyModifiers::NONE))
}

#[tokio::test]
async fn switching_tabs_keeps_each_tabs_position() {
    let (mut app, mut rx) = make_related_app(false);
    app.open_table("orders".to_string());
    settle(&mut app, &mut rx).await;
    app.update_grid(|grid| grid.focus_cell(2, 2));

    app.open_table("users".to_string());
    settle(&mut app, &mut rx).await;
    assert_eq!(app.grid.as_ref().expect("users grid").focused_row, 0);

    app.update(key(KeyCode::Char('[')));
    drain_messages(&mut app, &mut rx);
    let grid = app.grid.as_ref().expect("orders grid");
    assert_eq!(grid.table_name, "orders");
    assert_eq!(
        (grid.focused_row, grid.focused_col),
        (2, 2),
        "the tab keeps its focus"
    );
}

#[tokio::test]
async fn views_open_read_only() {
    let (mut app, mut rx) = make_related_app(false);
    app.open_table("big_orders".to_string());
    settle(&mut app, &mut rx).await;
    let grid = app.grid.as_ref().expect("view grid");
    assert!(grid.readonly);
    assert_eq!(grid.window.total_rows, 2);
    assert!(app.is_readonly_view());

    app.update(Message::OpenPopup);
    assert!(app.popup.is_none());
    assert_eq!(
        app.toast.toasts.back().map(|t| t.message.as_str()),
        Some("Views are read-only")
    );
}

#[tokio::test]
async fn readonly_flag_cannot_be_toggled_off() {
    let (mut app, _rx) = make_related_app(true);
    assert!(app.readonly_locked);
    app.execute_palette_command(PaletteCommand::ToggleReadonly);
    assert!(app.readonly, "the pool cannot write, so read-only stays on");
    assert!(app
        .toast
        .toasts
        .back()
        .is_some_and(|toast| toast.kind == ToastKind::Error));
}

#[tokio::test]
async fn referencing_rows_open_filtered_and_back_returns() {
    let (mut app, mut rx) = make_related_app(false);
    app.open_table("users".to_string());
    settle(&mut app, &mut rx).await;

    app.update(key(KeyCode::Char('r')));
    let Some(PopupKind::References(state)) = &app.popup else {
        panic!("expected the references popup");
    };
    assert_eq!(state.references.len(), 1);
    assert_eq!(state.references[0].table, "orders");
    assert_eq!(state.references[0].value, SqlValue::Integer(1));

    app.update(key(KeyCode::Enter));
    settle(&mut app, &mut rx).await;
    let grid = app.grid.as_ref().expect("orders grid");
    assert_eq!(grid.table_name, "orders");
    assert_eq!(grid.filter.active_count(), 1);
    assert_eq!(grid.window.total_rows, 2, "only Ada's orders");
    assert_eq!(app.jump_stack.len(), 1);
}

#[tokio::test]
async fn column_keys_hide_resize_and_add_sort_keys() {
    let (mut app, mut rx) = make_related_app(false);
    app.open_table("orders".to_string());
    settle(&mut app, &mut rx).await;
    render_test_app(&mut app);
    let before = app.grid.as_ref().expect("grid").col_widths[0];

    app.update(key(KeyCode::Char('>')));
    assert_eq!(app.grid.as_ref().expect("grid").col_widths[0], before + 2);

    app.update(key(KeyCode::Char('s')));
    app.update(key(KeyCode::Right));
    app.update(key(KeyCode::Char('S')));
    drain_messages(&mut app, &mut rx);
    assert_eq!(app.grid.as_ref().expect("grid").sort.len(), 2);

    app.update(key(KeyCode::Char('-')));
    let grid = app.grid.as_ref().expect("grid");
    assert_eq!(grid.display_columns(), vec![0, 2]);
    assert_eq!(
        grid.focused_col, 2,
        "focus moves to the next visible column"
    );
}

#[tokio::test]
async fn esc_focuses_the_sidebar_and_hiding_it_returns_focus() {
    let (mut app, mut rx) = make_related_app(false);
    app.open_table("users".to_string());
    settle(&mut app, &mut rx).await;
    assert!(matches!(app.focus, FocusPane::Grid));

    app.update(key(KeyCode::Esc));
    assert!(matches!(app.focus, FocusPane::Sidebar));

    app.update(Message::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('b'),
        KeyModifiers::CONTROL,
    )));
    assert!(!app.sidebar_visible);
    assert!(matches!(app.focus, FocusPane::Grid));
}

#[tokio::test]
async fn space_toggles_the_focused_row() {
    let (mut app, _rx) = make_test_app();
    app.grid = Some(make_grid());
    app.focus = FocusPane::Grid;
    app.update(key(KeyCode::Char(' ')));
    assert!(app.grid.as_ref().expect("grid").is_row_selected(0));
    app.update(key(KeyCode::Char(' ')));
    assert!(!app.grid.as_ref().expect("grid").has_row_selection());
}

#[tokio::test]
async fn sidebar_i_shows_the_schema() {
    let (mut app, mut rx) = make_related_app(false);
    app.focus = FocusPane::Sidebar;
    app.sidebar.move_down(&app.schema);
    app.update(key(KeyCode::Char('i')));
    while app.popup.is_none() {
        receive_one(&mut app, &mut rx).await;
    }
    let Some(PopupKind::Schema(state)) = &app.popup else {
        panic!("expected the schema view");
    };
    assert_eq!(state.name, "orders");
    assert!(state.ddl.contains("CREATE TABLE"));
}

#[tokio::test]
async fn copy_rows_as_csv_uses_the_selection() {
    let (mut app, mut rx) = make_related_app(false);
    app.open_table("users".to_string());
    settle(&mut app, &mut rx).await;
    app.update_grid(GridState::select_all_rows);

    app.copy_rows(CopyFormat::Csv);
    for _ in 0..5 {
        receive_one(&mut app, &mut rx).await;
        if app.toast.toasts.back().is_some() {
            break;
        }
    }
    assert_eq!(
        app.toast.toasts.back().map(|t| t.message.as_str()),
        Some("Copied 2 rows as CSV")
    );
}

#[tokio::test]
async fn unconfirmed_deletion_says_so_when_it_expires() {
    let (mut app, _rx) = make_test_app();
    app.pending_confirm = Some(PendingConfirm {
        message: "Delete?".to_string(),
        kind: ConfirmKind::DeleteRow {
            table: "users".to_string(),
            rowid: 1,
        },
        created: std::time::Instant::now() - std::time::Duration::from_secs(CONFIRM_TIMEOUT_SECS),
    });
    app.update(Message::Tick);
    assert!(app.pending_confirm.is_none());
    assert_eq!(
        app.toast.toasts.back().map(|t| t.message.as_str()),
        Some("Deletion not confirmed; nothing deleted")
    );
}

/// Every popup, drawn over a grid whose data holds control characters.
fn every_popup(app: &App) -> Vec<PopupKind> {
    use crate::ui::popup::{
        record::RecordInit, references::Reference, schema_view::SchemaLine, CommandPaletteState,
        DatePickerState, ExportState, FindState, FkPickerState, GlobalSearchState, GoToRowState,
        JsonViewState, RecordState, ReferencesState, SchemaViewState, SqlConsoleState,
        TextEditorState, ValuePickerState,
    };
    let grid = app.grid.as_ref().expect("grid");
    let evil = SqlValue::Text("a\x1b]2;x\x07\nb\tc".to_string());
    let long = "x".repeat(300);
    vec![
        PopupKind::TextEditor(TextEditorState::new(
            "users".into(),
            1,
            "name".into(),
            "TEXT".into(),
            SqlValue::Text(format!("{long}\n{long}")),
            false,
        )),
        PopupKind::ValuePicker(ValuePickerState::new(
            "users".into(),
            1,
            "name".into(),
            "TEXT".into(),
            vec![long.clone(), "a\x1bb".into()],
            evil.clone(),
        )),
        PopupKind::DatePicker(DatePickerState::datetime(
            "users".into(),
            1,
            "at".into(),
            SqlValue::Text("2026-09-30 12:00:00".into()),
        )),
        PopupKind::InsertRow(InsertRowState::new("users".into(), grid.columns.clone(), 0)),
        PopupKind::FkPicker(FkPickerState::new(
            "users".into(),
            grid.columns.clone(),
            "users".into(),
            "id".into(),
            1,
            SqlValue::Integer(1),
        )),
        PopupKind::FilterPopup(FilterPopupState::new(
            "name".into(),
            "TEXT".into(),
            Default::default(),
        )),
        PopupKind::CommandPalette(CommandPaletteState::new(vec![long.clone()])),
        PopupKind::Help(HelpState::new()),
        PopupKind::Find(FindState::new("users".into(), grid.columns.clone())),
        PopupKind::GoToRow(GoToRowState::new(50)),
        PopupKind::Record(RecordState::new(RecordInit {
            table: "users".into(),
            row_number: 1,
            names: grid.columns.iter().map(|c| c.name.clone()).collect(),
            kinds: grid.kinds.clone(),
            values: vec![
                SqlValue::Integer(1),
                evil.clone(),
                SqlValue::Null,
                SqlValue::Text("{\"a\":[1]}".into()),
            ],
            links: vec![false, true, false, false],
            editable: true,
            focused: 1,
        })),
        PopupKind::Schema(SchemaViewState::new(
            "users".into(),
            "CREATE TABLE users (\n id INTEGER\n)".into(),
            vec![
                SchemaLine::Heading("Columns".into()),
                SchemaLine::Text(long.clone()),
            ],
        )),
        PopupKind::SqlConsole(SqlConsoleState::new(vec!["SELECT 1".into()])),
        PopupKind::GlobalSearch(GlobalSearchState::new()),
        PopupKind::Export(ExportState::new(
            crate::export::ExportFormat::Csv,
            long.clone(),
            3,
        )),
        PopupKind::References(ReferencesState::new(
            "users".into(),
            vec![Reference {
                table: "orders".into(),
                column: "user_id".into(),
                value: evil,
            }],
        )),
        PopupKind::Json(JsonViewState::new(
            "doc".into(),
            serde_json::json!({"a": [1, {"b": long}]}),
        )),
    ]
}

#[tokio::test]
async fn every_popup_renders_at_any_terminal_size() {
    let (mut app, _rx) = make_test_app();
    let mut grid = make_grid();
    grid.window.rows[0][1] = SqlValue::Text("evil\x1b]2;PWNED\x07\ttab\nline".into());
    app.grid = Some(grid);
    app.open_tabs = vec![TableTab::new("users".to_string())];
    app.active_tab = Some(0);
    let popups = every_popup(&app);
    for popup in popups {
        app.popup = Some(popup);
        app.mode = AppMode::Edit;
        for (width, height) in [(1, 1), (20, 3), (30, 5), (40, 10), (80, 15), (120, 40)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
            terminal
                .draw(|frame| crate::ui::render(frame, &mut app))
                .expect("render");
            let buffer = terminal.backend().buffer();
            assert!(
                buffer
                    .content()
                    .iter()
                    .all(|cell| !cell.symbol().chars().any(char::is_control)),
                "control characters reached the terminal at {width}x{height}"
            );
        }
    }
}
