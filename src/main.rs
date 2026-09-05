mod app;
mod app_dirs;
mod config;
mod db;
mod event;
mod export;
mod filter;
mod grid;
mod symbols;
mod theme;
mod ui;

use std::{io::Write, sync::Arc};

use anyhow::Context;
use clap::Parser;
use ratatui::{backend::CrosstermBackend, Terminal};

use app::{App, Message};
use config::Config;

const CLI_AFTER_HELP: &str = "\
Examples:
  sqview path/to/database.db
  sqview :memory:
  sqview path/to/database.db --readonly --no-watch
  sqview check-terminal
  sqview paths";

#[derive(Parser, Debug)]
#[command(
    name = "sqview",
    about = "Keyboard-first terminal SQLite viewer",
    version,
    arg_required_else_help = true,
    args_conflicts_with_subcommands = true,
    disable_help_subcommand = true,
    after_help = CLI_AFTER_HELP
)]
struct Cli {
    /// Path to SQLite database file, or :memory:
    #[arg(value_name = "DB_PATH", value_parser = parse_database_path)]
    path: Option<String>,
    /// Open database in read-only mode
    #[arg(long, requires = "path")]
    readonly: bool,
    /// Disable automatic external refresh when the database file changes
    #[arg(long, requires = "path")]
    no_watch: bool,
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(clap::Subcommand, Debug, Clone, Copy, PartialEq, Eq)]
enum CliCommand {
    /// Check terminal capabilities
    CheckTerminal,
    /// Print the config and data paths sqview uses
    Paths,
}

#[derive(Debug, PartialEq, Eq)]
enum RunMode {
    Open(OpenOptions),
    CheckTerminal,
    Paths,
}

#[derive(Debug, PartialEq, Eq)]
struct OpenOptions {
    path: String,
    readonly: bool,
    watch: bool,
}

impl Cli {
    fn into_run_mode(self) -> RunMode {
        match self.command {
            Some(CliCommand::CheckTerminal) => RunMode::CheckTerminal,
            Some(CliCommand::Paths) => RunMode::Paths,
            None => RunMode::Open(OpenOptions {
                path: self
                    .path
                    .expect("clap should require a database path when no subcommand is present"),
                readonly: self.readonly,
                watch: !self.no_watch,
            }),
        }
    }
}

struct TerminalGuard;

impl TerminalGuard {
    fn new(theme: &theme::Theme) -> anyhow::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        if let Err(err) = set_terminal_background(theme) {
            let _ = crossterm::terminal::disable_raw_mode();
            return Err(err.into());
        }
        if let Err(err) = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::event::EnableMouseCapture,
            crossterm::cursor::Hide,
        ) {
            let _ = reset_terminal_background();
            let _ = crossterm::terminal::disable_raw_mode();
            return Err(err.into());
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn restore_terminal() {
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::cursor::Show,
    );
    let _ = reset_terminal_background();
}

fn set_terminal_background(theme: &theme::Theme) -> std::io::Result<()> {
    if let Some(osc) = terminal_background_osc(theme) {
        write_osc(&osc)?;
    }
    Ok(())
}

fn reset_terminal_background() -> std::io::Result<()> {
    write_osc("\x1b]111\x07")
}

fn write_osc(sequence: &str) -> std::io::Result<()> {
    let mut stdout = std::io::stdout();
    stdout.write_all(sequence.as_bytes())?;
    stdout.flush()
}

fn terminal_background_osc(theme: &theme::Theme) -> Option<String> {
    color_osc_spec(theme.bg).map(|color| format!("\x1b]11;{color}\x07"))
}

fn color_osc_spec(color: ratatui::style::Color) -> Option<String> {
    match color {
        ratatui::style::Color::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        _ => None,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().into_run_mode() {
        RunMode::Open(options) => open_database(options).await,
        RunMode::CheckTerminal => {
            check_terminal();
            Ok(())
        }
        RunMode::Paths => {
            print_paths();
            Ok(())
        }
    }
}

async fn open_database(options: OpenOptions) -> anyhow::Result<()> {
    let path = options.path.as_str();

    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        orig_hook(info);
    }));

    let config = Config::load().context("Failed to load sqview config")?;

    let pool = Arc::new(
        db::open_pool(path, options.readonly)
            .with_context(|| format!("Failed to open database '{}'", path))?,
    );

    let conn = pool.get().context("Failed to get DB connection")?;
    let schema = db::load_schema(&conn).context("Failed to load schema")?;
    drop(conn);

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

    let _watcher = create_file_watcher(path, options.watch, tx.clone())?;

    let mut app = App::new(schema, config, pool, tx, options.readonly, path.to_string());
    let _guard = TerminalGuard::new(&app.theme)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;

    run_event_loop(&mut terminal, &mut app, &mut rx).await?;

    Ok(())
}

fn parse_database_path(value: &str) -> Result<String, String> {
    if value == ":memory:" || std::path::Path::new(value).exists() {
        Ok(value.to_string())
    } else {
        Err(format!("database file '{value}' does not exist"))
    }
}

fn should_watch_database(path: &str, watch: bool) -> bool {
    watch && path != ":memory:"
}

fn create_file_watcher(
    path: &str,
    watch: bool,
    tx: tokio::sync::mpsc::UnboundedSender<Message>,
) -> anyhow::Result<Option<notify::RecommendedWatcher>> {
    if !should_watch_database(path, watch) {
        return Ok(None);
    }

    use notify::{EventKind, RecursiveMode, Watcher};

    let database = std::fs::canonicalize(path).unwrap_or_else(|_| std::path::PathBuf::from(path));
    let parent = database
        .parent()
        .ok_or_else(|| anyhow::anyhow!("database path has no parent directory"))?
        .to_path_buf();
    let database_name = database
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("database path has no filename"))?
        .to_string_lossy()
        .into_owned();
    let watched_names = [
        database_name.clone(),
        format!("{database_name}-wal"),
        format!("{database_name}-shm"),
    ];

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            if matches!(
                event.kind,
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
            ) && event.paths.iter().any(|path| {
                path.file_name().is_some_and(|name| {
                    watched_names.iter().any(|watched| name == watched.as_str())
                })
            }) {
                let _ = tx.send(Message::FileChanged);
            }
        }
    })?;
    watcher.watch(&parent, RecursiveMode::NonRecursive)?;
    Ok(Some(watcher))
}

async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<Message>,
) -> anyhow::Result<()> {
    use futures::StreamExt;

    let mut events = crossterm::event::EventStream::new();
    let mut tick_interval = tokio::time::interval(std::time::Duration::from_millis(33));

    terminal.draw(|f| app.view(f))?;
    app.dirty = false;

    loop {
        tokio::select! {
            _ = tick_interval.tick() => {
                app.update(Message::Tick);
                if app.should_quit {
                    break;
                }
                if app.dirty {
                    terminal.draw(|f| app.view(f))?;
                    app.dirty = false;
                }
            }
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(ev)) => {
                        if let Some(msg) = event::translate_event(ev) {
                            app.update(msg);
                        }
                    }
                    _ => break,
                }
            }
            Some(msg) = rx.recv() => {
                app.update(msg);
            }
        }
    }

    Ok(())
}

fn print_paths() {
    print_path_line("config", crate::app_dirs::config_file());
    print_path_line("data", crate::app_dirs::data_local_dir());
    print_path_line("filters", crate::app_dirs::filter_dir());
}

fn print_path_line(label: &str, path: Option<std::path::PathBuf>) {
    match path {
        Some(path) => println!("{label}: {}", path.display()),
        None => println!("{label}: unavailable"),
    }
}

fn check_terminal() {
    let colorterm = std::env::var("COLORTERM").unwrap_or_else(|_| "unknown".to_string());
    let color_support = match colorterm.to_lowercase().as_str() {
        "truecolor" | "24bit" => "truecolor ✓",
        "256color" => "256color",
        _ => "unknown",
    };

    let term_program = std::env::var("TERM_PROGRAM").unwrap_or_else(|_| "unknown".to_string());
    let term = std::env::var("TERM").unwrap_or_default();
    let mouse = if term.contains("xterm") || !term_program.is_empty() {
        "yes"
    } else {
        "unknown"
    };

    println!("Terminal capabilities:");
    println!("  COLORTERM: {}", color_support);
    println!(
        "  TERM_PROGRAM: {}",
        if term_program.is_empty() {
            "unknown".to_string()
        } else {
            term_program
        }
    );
    println!("  Mouse support: {} (from TERM)", mouse);
    println!("  Unicode: yes");
    println!("  Nerd fonts: not detectable (set nerd_font = false in config if icons look wrong)");
}

#[cfg(test)]
mod tests {
    use clap::error::ErrorKind;

    use super::*;

    #[test]
    fn help_lists_open_flags_and_subcommands() {
        let err = Cli::try_parse_from(["sqview", "--help"]).expect_err("help should exit");
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
        let help = err.to_string();
        assert!(help.contains("--no-watch"));
        assert!(help.contains("check-terminal"));
        assert!(help.contains("paths"));
    }

    #[test]
    fn version_flag_is_available() {
        let err = Cli::try_parse_from(["sqview", "--version"]).expect_err("version should exit");
        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
        assert!(err.to_string().contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn missing_args_show_helpful_usage() {
        let err = Cli::try_parse_from(["sqview"]).expect_err("missing args should fail");
        let rendered = err.to_string();
        assert!(rendered.contains("Usage:"));
        assert!(rendered.contains("DB_PATH"));
    }

    #[test]
    fn open_mode_routes_runtime_flags() {
        let cli = Cli::try_parse_from(["sqview", ":memory:", "--readonly", "--no-watch"]).unwrap();
        assert_eq!(
            cli.into_run_mode(),
            RunMode::Open(OpenOptions {
                path: ":memory:".to_string(),
                readonly: true,
                watch: false,
            })
        );
    }

    #[test]
    fn paths_subcommand_routes_without_db_path() {
        let cli = Cli::try_parse_from(["sqview", "paths"]).unwrap();
        assert_eq!(cli.into_run_mode(), RunMode::Paths);
    }

    #[test]
    fn missing_database_file_is_rejected_by_clap() {
        let missing = std::env::temp_dir().join(format!(
            "sqview-cli-missing-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock before unix epoch")
                .as_nanos()
        ));
        let missing = missing.display().to_string();

        let err = Cli::try_parse_from(["sqview", &missing]).expect_err("missing DB should fail");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn watch_helper_skips_memory_and_opt_out() {
        assert!(!should_watch_database(":memory:", true));
        assert!(!should_watch_database("demo.db", false));
        assert!(should_watch_database("demo.db", true));
    }

    #[test]
    fn terminal_background_osc_uses_theme_background() {
        let theme = crate::theme::Theme::default();
        assert_eq!(
            terminal_background_osc(&theme),
            Some("\u{1b}]11;#23211f\u{7}".to_string())
        );
    }
}
