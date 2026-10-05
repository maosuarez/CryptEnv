//! `crypt-env tui` — keyboard-driven terminal UI (ratatui) mirroring the CLI
//! project workflow: browse projects → environments → variables (values
//! masked), `/` filter, password-gated reveal (`v`), and the workspace
//! actions `init`, `config`, `fill`, `sync` and `doctor` on the current
//! directory's `.crypt-env.yaml`.

use clap::Args;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    cursor::Show,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use std::cell::{Cell, RefCell};
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use zeroize::{Zeroize, Zeroizing};

use crate::client::{self, CliError, ItemSummary, Project, VaultApi};
use crate::commands::{config, doctor, fill, init, scope, search, sync};

// ─── Palette ──────────────────────────────────────────────────────────────────

const BG: Color = Color::Rgb(18, 18, 22);
const SURFACE: Color = Color::Rgb(24, 24, 30);
const RAISED: Color = Color::Rgb(30, 30, 38);
const ACCENT: Color = Color::Rgb(0, 200, 150);
const TX: Color = Color::Rgb(220, 220, 225);
const TX2: Color = Color::Rgb(150, 150, 160);
const TX3: Color = Color::Rgb(90, 90, 100);
const DANGER: Color = Color::Rgb(240, 80, 80);
const WARN: Color = Color::Rgb(255, 200, 60);

#[derive(Args)]
pub struct TuiArgs {}

// ─── State ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Projects,
    Environments,
    Variables,
}

/// What a password prompt unlocks once verified. Every action that reaches
/// the vault goes through [`gate`] with one of these, so the client never has
/// to prompt on the raw terminal.
#[derive(Clone, PartialEq, Eq)]
enum Gated {
    Login,
    Reload,
    Reveal,
    Fill,
    Sync { global: bool },
    /// `config`: plan, and ask for confirmation when it changes secret routing.
    Config,
    /// `config` after the user confirmed the diff.
    ConfigPush,
    /// `init` for this project name.
    Init { name: String },
    /// `init` after the user confirmed adopting a rootless project.
    InitAdopt { name: String },
}

/// Longest password the modal accepts, in bytes. The input buffer is allocated
/// at exactly this capacity up front and never grows, so typing cannot leave
/// reallocated (and therefore unwiped) copies of a partial password behind.
const PASSWORD_CAP: usize = 256;

/// A fresh, pre-allocated, wiped-on-drop password input buffer.
fn new_password_input() -> Zeroizing<String> {
    Zeroizing::new(String::with_capacity(PASSWORD_CAP))
}

/// Status shown while a blocking backend call is pending.
const WORKING: &str = "working…";

/// A blocking action queued so the event loop can draw [`WORKING`] first.
enum Pending {
    Gate(Gated),
    Submit(Gated, Zeroizing<String>),
    Doctor,
}

enum Modal {
    None,
    Password { purpose: Gated, input: Zeroizing<String>, error: Option<String> },
    Input { title: &'static str, value: String },
    /// A secret-routing change awaiting `y`; `then` runs through [`gate`].
    Confirm { title: String, lines: Vec<String>, then: Gated },
    Reveal { key: String, value: Zeroizing<String> },
    Report { title: String, lines: Vec<(String, Color)> },
    Help,
}

/// One row of the variables pane.
struct VarRow {
    key: String,
    item_id: i64,
    detail: String,
}

struct App {
    api: Box<dyn VaultApi>,
    projects: Vec<Project>,
    globals: Vec<ItemSummary>,
    pane: Pane,
    proj_state: ListState,
    env_state: ListState,
    var_state: ListState,
    filter: String,
    filtering: bool,
    modal: Modal,
    status: Option<(String, bool)>,
    quit: bool,
    workspace_dir: PathBuf,
    /// Cached `find_config` result, so idle redraws never touch the
    /// filesystem (slow over 9P on `/mnt/c`). Dropped by [`App::reload`] and
    /// before every workspace action.
    config_path: RefCell<Option<Option<PathBuf>>>,
    /// Compiled filter, cached per filter string.
    matcher: RefCell<Option<(String, Rc<search::Matcher>)>>,
    /// Queue blocking actions for the event loop instead of running them
    /// inline (off in unit tests, which run reducers synchronously).
    defer: bool,
    pending: Option<Pending>,
    /// Counters for the cache tests.
    config_lookups: Cell<u32>,
    matcher_builds: Cell<u32>,
}

impl App {
    fn new(workspace_dir: PathBuf, api: Box<dyn VaultApi>) -> Self {
        App {
            api,
            projects: Vec::new(),
            globals: Vec::new(),
            pane: Pane::Projects,
            proj_state: ListState::default(),
            env_state: ListState::default(),
            var_state: ListState::default(),
            filter: String::new(),
            filtering: false,
            modal: Modal::None,
            status: None,
            quit: false,
            workspace_dir,
            config_path: RefCell::new(None),
            matcher: RefCell::new(None),
            defer: false,
            pending: None,
            config_lookups: Cell::new(0),
            matcher_builds: Cell::new(0),
        }
    }

    /// Nearest manifest from the workspace dir, resolved once until
    /// [`App::forget_config_path`].
    fn config_path(&self) -> Option<PathBuf> {
        self.config_path
            .borrow_mut()
            .get_or_insert_with(|| {
                self.config_lookups.set(self.config_lookups.get() + 1);
                scope::find_config(&self.workspace_dir)
            })
            .clone()
    }

    fn forget_config_path(&self) {
        *self.config_path.borrow_mut() = None;
    }

    /// Matcher for the current filter; recompiled only when the filter text changes.
    fn matcher(&self) -> Rc<search::Matcher> {
        let mut cache = self.matcher.borrow_mut();
        match cache.as_ref() {
            Some((f, m)) if *f == self.filter => m.clone(),
            _ => {
                self.matcher_builds.set(self.matcher_builds.get() + 1);
                let m = Rc::new(search::Matcher::new(if self.filter.is_empty() { None } else { Some(&self.filter) }));
                *cache = Some((self.filter.clone(), m.clone()));
                m
            }
        }
    }

    /// Runs `p` now, or queues it (showing [`WORKING`]) for the event loop.
    fn schedule(&mut self, p: Pending) {
        if self.defer {
            self.status = Some((WORKING.into(), false));
            self.pending = Some(p);
        } else {
            run_pending(self, p);
        }
    }

    fn project(&self) -> Option<&Project> {
        self.proj_state.selected().and_then(|i| self.projects.get(i))
    }

    /// Environments pane entries: the project's environments plus a trailing
    /// "global" pseudo-entry listing reusable global items.
    fn env_labels(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .project()
            .map(|p| {
                p.environments
                    .iter()
                    .map(|e| {
                        let n = if e.name.is_empty() { "(root .env)".to_string() } else { e.name.clone() };
                        if e.is_default { format!("{n} *") } else { n }
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.push("★ global items".into());
        v
    }

    fn global_selected(&self) -> bool {
        let n = self.project().map(|p| p.environments.len()).unwrap_or(0);
        self.env_state.selected() == Some(n)
    }

    fn rows(&self) -> Vec<VarRow> {
        let m = self.matcher();
        if self.global_selected() {
            return self
                .globals
                .iter()
                .filter_map(|g| {
                    let key = g.name.clone().or(g.title.clone())?;
                    m.is_match(&key).then(|| VarRow { key, item_id: g.id, detail: g.item_type.clone() })
                })
                .collect();
        }
        let Some(env) = self.project().and_then(|p| self.env_state.selected().and_then(|i| p.environments.get(i))) else {
            return Vec::new();
        };
        env.vars
            .iter()
            .filter(|v| m.is_match(&v.key))
            .map(|v| VarRow { key: v.key.clone(), item_id: v.item_id, detail: format!("item #{}", v.item_id) })
            .collect()
    }

    fn reload(&mut self) -> Result<(), CliError> {
        let selected = self.project().map(|p| p.id);
        self.projects = self.api.fetch_projects()?;
        self.forget_config_path();
        let ws_name = self
            .config_path()
            .and_then(|p| scope::load_config(&p).ok())
            .map(|w| w.project.to_lowercase());
        let idx = selected
            .and_then(|id| self.projects.iter().position(|p| p.id == id))
            .or_else(|| ws_name.and_then(|n| self.projects.iter().position(|p| p.name.to_lowercase() == n)))
            .or(if self.projects.is_empty() { None } else { Some(0) });
        self.proj_state.select(idx);
        self.globals = match self.projects.iter().find(|p| !p.environments.is_empty()) {
            Some(p) => {
                let env = p.environments.iter().find(|e| e.is_default).unwrap_or(&p.environments[0]);
                self.api.list_items(&p.name, &env.name, "only")?
            }
            None => Vec::new(),
        };
        self.reset_env();
        Ok(())
    }

    fn reset_env(&mut self) {
        let default = self.project().and_then(|p| p.environments.iter().position(|e| e.is_default)).unwrap_or(0);
        self.env_state.select(Some(default));
        self.reset_vars();
    }

    fn reset_vars(&mut self) {
        let n = self.rows().len();
        self.var_state.select(if n == 0 { None } else { Some(0) });
    }

    fn report(&mut self, title: impl Into<String>, lines: Vec<(String, Color)>) {
        self.modal = Modal::Report { title: title.into(), lines };
    }

    fn error(&mut self, e: CliError) {
        self.report("Error", vec![(e.to_string(), DANGER)]);
    }

    /// Reports `e`; a session that lapsed mid-action (`SessionRequired`)
    /// instead opens the password modal for `purpose`.
    fn fail(&mut self, e: CliError, purpose: Gated) {
        match e {
            CliError::SessionRequired => {
                self.modal = Modal::Password { purpose, input: new_password_input(), error: None };
            }
            other => self.error(other),
        }
    }
}

// ─── Entry / loop ─────────────────────────────────────────────────────────────

/// Restores the terminal when dropped: leave the alternate screen, show the
/// cursor, disable raw mode, hand the terminal back to the client. Created
/// right after raw mode is enabled so every later exit path restores it.
struct TerminalGuard<F: Fn()>(F);

impl<F: Fn()> Drop for TerminalGuard<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

fn restore_terminal() {
    client::set_non_interactive(false);
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}

/// Enables raw mode, then builds the terminal. The guard exists before
/// `build` runs, so a `build` failure still restores the terminal.
fn open_terminal<T, R: Fn()>(
    enable_raw: impl FnOnce() -> io::Result<()>,
    build: impl FnOnce() -> Result<T, CliError>,
    restore: R,
) -> Result<(TerminalGuard<R>, T), CliError> {
    enable_raw().map_err(CliError::Io)?;
    let guard = TerminalGuard(restore);
    let terminal = build()?;
    Ok((guard, terminal))
}

pub fn run(_args: TuiArgs) -> Result<(), CliError> {
    let cwd = std::env::current_dir()?;
    let (_guard, mut terminal) = open_terminal(
        enable_raw_mode,
        || {
            let mut stdout = io::stdout();
            execute!(stdout, EnterAlternateScreen).map_err(CliError::Io)?;
            Terminal::new(CrosstermBackend::new(stdout)).map_err(|e| CliError::Api(e.to_string()))
        },
        restore_terminal,
    )?;

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    // From here the TUI owns the terminal: the client must never prompt on it.
    client::set_non_interactive(true);

    let mut app = App::new(cwd, Box::new(client::Live));
    app.status = Some((WORKING.into(), false));
    let _ = terminal.draw(|f| render(f, &app));
    if !matches!(app.api.session_alive(), Ok(true)) || app.reload().is_err() {
        app.modal = Modal::Password { purpose: Gated::Login, input: new_password_input(), error: None };
    }
    app.status = None;
    app.defer = true;
    run_loop(&mut terminal, &mut app)
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<(), CliError> {
    while !app.quit {
        terminal.draw(|f| render(f, app)).map_err(|e| CliError::Api(e.to_string()))?;
        // The frame above already shows `working…`; run what it announced.
        if let Some(p) = app.pending.take() {
            run_pending(app, p);
            continue;
        }
        if event::poll(Duration::from_millis(100)).map_err(CliError::Io)? {
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind == KeyEventKind::Press {
                    handle_key(app, key);
                }
            }
        }
    }
    Ok(())
}

// ─── Input ────────────────────────────────────────────────────────────────────

fn handle_key(app: &mut App, key: KeyEvent) {
    // Control combinations are never plain letters. Ctrl+C always quits, in
    // every state including modals. AltGr is reported as Ctrl+Alt on Windows
    // and types real characters, so Ctrl together with Alt is not "control".
    if key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) {
        if matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C')) {
            if let Modal::Password { input, .. } = &mut app.modal {
                input.zeroize();
            }
            app.quit = true;
        }
        return;
    }
    let code = key.code;
    match &mut app.modal {
        Modal::None => {}
        Modal::Password { input, purpose, .. } => {
            match code {
                KeyCode::Char(c) => {
                    // Refuse input past the pre-allocated capacity (no realloc).
                    if input.len() + c.len_utf8() <= PASSWORD_CAP {
                        input.push(c);
                    }
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Esc if *purpose == Gated::Login => {
                    input.zeroize();
                    app.quit = true;
                }
                KeyCode::Esc => {
                    input.zeroize();
                    app.modal = Modal::None;
                }
                KeyCode::Enter => {
                    let purpose = purpose.clone();
                    // Moves the single allocation out; the modal gets a fresh buffer.
                    let pw = std::mem::replace(input, new_password_input());
                    app.schedule(Pending::Submit(purpose, pw));
                }
                _ => {}
            }
            return;
        }
        Modal::Confirm { then, .. } => {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let then = then.clone();
                    app.modal = Modal::None;
                    gate(app, then);
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => app.modal = Modal::None,
                _ => {}
            }
            return;
        }
        Modal::Input { value, .. } => {
            match code {
                KeyCode::Char(c) => value.push(c),
                KeyCode::Backspace => {
                    value.pop();
                }
                KeyCode::Esc => app.modal = Modal::None,
                KeyCode::Enter => {
                    let name = std::mem::take(value);
                    app.modal = Modal::None;
                    gate(app, Gated::Init { name });
                }
                _ => {}
            }
            return;
        }
        // Reveal / report / help: any key dismisses (the revealed value is
        // zeroized on drop).
        _ => {
            app.modal = Modal::None;
            return;
        }
    }

    if app.filtering {
        match code {
            KeyCode::Char(c) => app.filter.push(c),
            KeyCode::Backspace => {
                app.filter.pop();
            }
            KeyCode::Esc => {
                app.filter.clear();
                app.filtering = false;
            }
            KeyCode::Enter => app.filtering = false,
            _ => {}
        }
        app.reset_vars();
        return;
    }

    match code {
        KeyCode::Char('q') => app.quit = true,
        KeyCode::Char('?') => app.modal = Modal::Help,
        KeyCode::Char('/') => {
            app.pane = Pane::Variables;
            app.filtering = true;
        }
        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
            app.pane = match app.pane {
                Pane::Projects => Pane::Environments,
                _ => Pane::Variables,
            }
        }
        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
            app.pane = match app.pane {
                Pane::Variables => Pane::Environments,
                _ => Pane::Projects,
            }
        }
        KeyCode::Down | KeyCode::Char('j') => step(app, 1),
        KeyCode::Up | KeyCode::Char('k') => step(app, -1),
        KeyCode::Enter => {
            app.pane = match app.pane {
                Pane::Projects => Pane::Environments,
                _ => Pane::Variables,
            }
        }
        KeyCode::Char('v') => {
            if app.var_state.selected().is_some() && !app.rows().is_empty() {
                gate(app, Gated::Reveal);
            }
        }
        KeyCode::Char('r') => gate(app, Gated::Reload),
        KeyCode::Char('i') => {
            let default = app.workspace_dir.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            app.modal = Modal::Input { title: " init — project name (Enter to confirm) ", value: default };
        }
        KeyCode::Char('c') => gate(app, Gated::Config),
        KeyCode::Char('f') => gate(app, Gated::Fill),
        KeyCode::Char('s') => gate(app, Gated::Sync { global: false }),
        KeyCode::Char('S') => gate(app, Gated::Sync { global: true }),
        KeyCode::Char('d') => app.schedule(Pending::Doctor),
        _ => {}
    }
}

/// Executes a queued action, then clears the `working…` status it set.
fn run_pending(app: &mut App, p: Pending) {
    match p {
        Pending::Gate(purpose) => gate_now(app, purpose),
        Pending::Submit(purpose, pw) => submit_password(app, purpose, &pw),
        Pending::Doctor => {
            let lines = doctor::execute()
                .into_iter()
                .map(|c| {
                    let color = match c.level {
                        doctor::Level::Ok => ACCENT,
                        doctor::Level::Warn => WARN,
                        doctor::Level::Fail => DANGER,
                    };
                    (format!("{:<18} {}", c.name, c.detail), color)
                })
                .collect();
            app.report("doctor", lines);
        }
    }
    if matches!(&app.status, Some((m, _)) if m == WORKING) {
        app.status = None;
    }
}

fn step(app: &mut App, delta: i32) {
    let (state, len) = match app.pane {
        Pane::Projects => (&mut app.proj_state, app.projects.len()),
        Pane::Environments => {
            let n = app.env_labels().len();
            (&mut app.env_state, n)
        }
        Pane::Variables => {
            let n = app.rows().len();
            (&mut app.var_state, n)
        }
    };
    if len == 0 {
        return;
    }
    let cur = state.selected().unwrap_or(0) as i32;
    state.select(Some((cur + delta).rem_euclid(len as i32) as usize));
    match app.pane {
        Pane::Projects => app.reset_env(),
        Pane::Environments => app.reset_vars(),
        Pane::Variables => {}
    }
}

/// Runs a gated action directly while this terminal's session is live (the
/// check renews it); otherwise asks for the master password first.
fn gate(app: &mut App, purpose: Gated) {
    app.schedule(Pending::Gate(purpose));
}

fn gate_now(app: &mut App, purpose: Gated) {
    match app.api.session_alive() {
        Ok(true) => run_gated(app, purpose),
        Ok(false) => app.modal = Modal::Password { purpose, input: new_password_input(), error: None },
        Err(e) => app.error(e),
    }
}

fn submit_password(app: &mut App, purpose: Gated, pw: &str) {
    if let Err(e) = app.api.authenticate(pw) {
        app.modal = Modal::Password { purpose, input: new_password_input(), error: Some(e.to_string()) };
        return;
    }
    app.modal = Modal::None;
    run_gated(app, purpose);
}

fn run_gated(app: &mut App, purpose: Gated) {
    match purpose.clone() {
        Gated::Login => {
            if let Err(e) = app.reload() {
                app.fail(e, purpose);
            }
        }
        Gated::Reload => match app.reload() {
            Ok(()) => app.status = Some(("Reloaded".into(), false)),
            Err(e) => app.fail(e, purpose),
        },
        Gated::Reveal => {
            let rows = app.rows();
            let Some(row) = app.var_state.selected().and_then(|i| rows.get(i)) else { return };
            match app.api.reveal_item(row.item_id) {
                Ok(value) => app.modal = Modal::Reveal { key: row.key.clone(), value },
                Err(e) => app.fail(e, purpose),
            }
        }
        Gated::Fill => match workspace(app).and_then(|ws| fill::execute(&*app.api, &ws, None)) {
            Ok(r) => {
                let mut lines: Vec<(String, Color)> = r.lines.into_iter().map(|l| (l, TX)).collect();
                lines.extend(r.examples.into_iter().map(|e| (format!("wrote {}", e.display()), TX2)));
                app.report("fill", lines);
            }
            Err(e) => app.fail(e, purpose),
        },
        Gated::Sync { global } => {
            let result = workspace(app).and_then(|ws| {
                let example = sync::locate_example(None, &ws.root)?;
                sync::execute(&ws, &example, None, global)
            });
            match result {
                Ok(r) => {
                    let mut lines = vec![(format!("read {}", r.example.display()), TX2)];
                    lines.push((format!("created (change-me): {}", join_or_none(&r.created)), TX));
                    lines.push((format!("linked to globals: {}", join_or_none(&r.linked)), TX));
                    if !r.written.is_empty() {
                        lines.push((format!("wrote {}", r.written.join(", ")), TX));
                    }
                    app.report(if global { "sync --global" } else { "sync" }, lines);
                    let _ = app.reload();
                }
                Err(e) => app.fail(e, purpose),
            }
        }
        Gated::Config => run_config(app, false),
        Gated::ConfigPush => run_config(app, true),
        Gated::Init { name } => {
            let dir = app.workspace_dir.clone();
            match init::check(&*app.api, &dir, Some(&name)) {
                Ok(init::Existing::Adopt { lines, .. }) => {
                    app.modal = Modal::Confirm {
                        title: " init — bind existing project to this directory? ".into(),
                        lines,
                        then: Gated::InitAdopt { name },
                    };
                }
                Ok(_) => run_init(app, &name, false),
                Err(e) => app.fail(e, purpose),
            }
        }
        Gated::InitAdopt { name } => run_init(app, &name, true),
    }
}

fn join_or_none(v: &[String]) -> String {
    if v.is_empty() { "none".into() } else { v.join(", ") }
}

/// The cwd workspace, without the legacy-notice `eprintln!` of
/// `scope::workspace` (which would corrupt the alternate screen).
fn workspace(app: &App) -> Result<scope::Workspace, CliError> {
    // User-driven action: look again, the manifest may have appeared or moved.
    app.forget_config_path();
    let path = app.config_path().ok_or_else(|| {
        CliError::Config(format!("no {} here — press `i` to init", scope::manifest::FILE_NAME))
    })?;
    scope::load_config(&path)
}

fn run_init(app: &mut App, name: &str, adopt_confirmed: bool) {
    let dir = app.workspace_dir.clone();
    match init::execute(&*app.api, &dir, Some(name), None, adopt_confirmed) {
        Ok(r) => {
            let verb = if r.created { "created" } else { "linked" };
            let mut lines = vec![
                (format!("{verb} project '{}' → {}", r.project, r.target), TX),
                (format!("wrote {}", r.manifest_path.display()), TX2),
            ];
            if r.gitignore == init::GitignoreUpdate::Added {
                lines.push((format!("added {} to .gitignore", scope::manifest::FILE_NAME), TX2));
            }
            app.report("init", lines);
            let _ = app.reload();
        }
        Err(e) => app.fail(e, Gated::Init { name: name.to_string() }),
    }
}

/// `config`. Unconfirmed, a secret-routing change opens the confirm modal
/// instead of being applied; `confirmed` (the user pressed `y`) applies it.
fn run_config(app: &mut App, confirmed: bool) {
    let purpose = if confirmed { Gated::ConfigPush } else { Gated::Config };
    let loaded = workspace(app).and_then(|ws| match ws.manifest.clone() {
        Some(m) => Ok((ws, m)),
        None => Err(CliError::Config("legacy crypt-env.json — press `i` to init".into())),
    });
    let (ws, m) = match loaded {
        Ok(v) => v,
        Err(e) => return app.fail(e, purpose),
    };
    let result = if confirmed {
        config::execute_confirmed(&*app.api, &ws.root, &ws.config_path, &m)
    } else {
        match config::prepare(&*app.api, &ws.root, &ws.config_path, &m, false) {
            Ok(prep) if prep.needs_consent() => {
                app.modal = Modal::Confirm {
                    title: " config — apply these changes to the vault? ".into(),
                    lines: prep.diff_lines(&m.project.name),
                    then: Gated::ConfigPush,
                };
                return;
            }
            Ok(prep) => config::apply(&*app.api, &ws.root, &m, &prep),
            Err(e) => Err(e),
        }
    };
    match result {
        Ok(config::Outcome::Pushed { untracked_envs, .. }) => {
            let mut lines = vec![("vault updated from .crypt-env.yaml".to_string(), TX)];
            if !untracked_envs.is_empty() {
                lines.push((format!("kept vault-only environments: {}", untracked_envs.join(", ")), WARN));
            }
            app.report("config", lines);
            let _ = app.reload();
        }
        Ok(config::Outcome::Pulled) => app.report("config", vec![(".crypt-env.yaml updated from the vault".into(), TX)]),
        Ok(config::Outcome::InSync) => app.report("config", vec![("already in sync".into(), TX)]),
        Err(e) => app.fail(e, purpose),
    }
}

// ─── Rendering ────────────────────────────────────────────────────────────────

fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(BG)), area);
    let [top, body, bottom] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)]).areas(area);

    let ws = app
        .config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "no .crypt-env.yaml here".into());
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" crypt-env ", Style::default().fg(BG).bg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {ws}"), Style::default().fg(TX2)),
        ]))
        .style(Style::default().bg(SURFACE)),
        top,
    );

    let [left, mid, right] =
        Layout::horizontal([Constraint::Percentage(28), Constraint::Percentage(24), Constraint::Percentage(48)]).areas(body);
    let proj_items: Vec<ListItem> = app.projects.iter().map(|p| ListItem::new(p.name.clone())).collect();
    render_list(f, left, " Projects ", proj_items, &app.proj_state, app.pane == Pane::Projects);
    let env_items: Vec<ListItem> = app.env_labels().into_iter().map(ListItem::new).collect();
    render_list(f, mid, " Environments ", env_items, &app.env_state, app.pane == Pane::Environments);
    let var_items: Vec<ListItem> = app
        .rows()
        .into_iter()
        .map(|r| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<28}", r.key), Style::default().fg(TX)),
                Span::styled("••••••••  ", Style::default().fg(TX3)),
                Span::styled(r.detail, Style::default().fg(TX3)),
            ]))
        })
        .collect();
    let title = if app.filter.is_empty() && !app.filtering {
        " Variables ".to_string()
    } else {
        format!(" Variables  /{}{} ", app.filter, if app.filtering { "▏" } else { "" })
    };
    render_list(f, right, &title, var_items, &app.var_state, app.pane == Pane::Variables);

    let hint = match &app.status {
        Some((m, true)) => Span::styled(format!(" {m}"), Style::default().fg(DANGER)),
        Some((m, false)) => Span::styled(format!(" {m}"), Style::default().fg(ACCENT)),
        None => Span::styled(
            " ←/→ pane  ↑/↓ move  / filter  v reveal  i init  c config  f fill  s/S sync  d doctor  r reload  ? help  q quit",
            Style::default().fg(TX3),
        ),
    };
    f.render_widget(Paragraph::new(Line::from(hint)).style(Style::default().bg(SURFACE)), bottom);

    render_modal(f, app, area);
}

fn render_list(f: &mut Frame, area: Rect, title: &str, items: Vec<ListItem>, state: &ListState, focused: bool) {
    let border = if focused { ACCENT } else { TX3 };
    let list = List::new(items)
        .block(
            Block::default()
                .title(Span::styled(title.to_string(), Style::default().fg(if focused { ACCENT } else { TX2 })))
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .border_style(Style::default().fg(border))
                .style(Style::default().bg(SURFACE)),
        )
        .style(Style::default().fg(TX))
        .highlight_style(Style::default().bg(RAISED).fg(ACCENT).add_modifier(Modifier::BOLD))
        .highlight_symbol("› ");
    let mut s = state.clone();
    f.render_stateful_widget(list, area, &mut s);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn modal_block(title: &str, color: Color) -> Block<'static> {
    Block::default()
        .title(Span::styled(title.to_string(), Style::default().fg(color).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color))
        .style(Style::default().bg(RAISED))
}

fn render_modal(f: &mut Frame, app: &App, area: Rect) {
    match &app.modal {
        Modal::None => {}
        Modal::Password { purpose, input, error } => {
            let r = centered(area, 60, 7);
            let what = match purpose {
                Gated::Login => "open a session in this terminal",
                Gated::Reload => "reload vault data",
                Gated::Reveal => "reveal this value",
                Gated::Fill => "fill .env files",
                Gated::Sync { global: false } => "sync .env.example",
                Gated::Sync { global: true } => "sync --global",
                Gated::Config | Gated::ConfigPush => "sync .crypt-env.yaml",
                Gated::Init { .. } | Gated::InitAdopt { .. } => "init this project",
            };
            let mut lines = vec![
                Line::from(Span::styled(format!("Master password to {what}:"), Style::default().fg(TX2))),
                Line::from(Span::styled(format!("{}▏", "•".repeat(input.chars().count())), Style::default().fg(ACCENT))),
            ];
            if let Some(e) = error {
                lines.push(Line::from(Span::styled(e.clone(), Style::default().fg(DANGER))));
            }
            lines.push(Line::from(Span::styled("Enter confirm · Esc cancel", Style::default().fg(TX3))));
            f.render_widget(Clear, r);
            f.render_widget(Paragraph::new(lines).block(modal_block(" password ", ACCENT)).wrap(Wrap { trim: true }), r);
        }
        Modal::Confirm { title, lines, .. } => {
            let h = (lines.len() as u16 + 3).min(area.height.saturating_sub(2));
            let r = centered(area, 96, h);
            let mut body: Vec<Line> =
                lines.iter().map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(TX)))).collect();
            body.push(Line::from(Span::styled("y apply · n/Esc cancel", Style::default().fg(WARN))));
            f.render_widget(Clear, r);
            f.render_widget(Paragraph::new(body).block(modal_block(title, WARN)).wrap(Wrap { trim: false }), r);
        }
        Modal::Input { title, value } => {
            let r = centered(area, 60, 5);
            f.render_widget(Clear, r);
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(format!("{value}▏"), Style::default().fg(TX))),
                    Line::from(Span::styled(
                        format!("root: {}", app.workspace_dir.display()),
                        Style::default().fg(TX3),
                    )),
                ])
                .block(modal_block(title, ACCENT)),
                r,
            );
        }
        Modal::Reveal { key, value } => {
            let r = centered(area, 72, 7);
            f.render_widget(Clear, r);
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(key.clone(), Style::default().fg(TX2))),
                    Line::from(Span::styled(value.as_str(), Style::default().fg(WARN))),
                    Line::from(Span::styled("any key to hide (value is cleared from crypt-env's memory; the terminal may keep it in scrollback)", Style::default().fg(TX3))),
                ])
                .block(modal_block(" revealed ", WARN))
                .wrap(Wrap { trim: false }),
                r,
            );
        }
        Modal::Report { title, lines } => {
            let h = (lines.len() as u16 + 3).min(area.height.saturating_sub(2));
            let r = centered(area, 96, h);
            let mut body: Vec<Line> =
                lines.iter().map(|(l, c)| Line::from(Span::styled(l.clone(), Style::default().fg(*c)))).collect();
            body.push(Line::from(Span::styled("any key to close", Style::default().fg(TX3))));
            f.render_widget(Clear, r);
            f.render_widget(Paragraph::new(body).block(modal_block(&format!(" {title} "), ACCENT)).wrap(Wrap { trim: false }), r);
        }
        Modal::Help => {
            let rows = [
                ("←/→ h/l, Tab", "switch pane (projects · environments · variables)"),
                ("↑/↓ j/k", "move selection"),
                ("/", "filter variables (regex, or %substring)"),
                ("v", "reveal value (master password)"),
                ("i", "init: register this directory as a project"),
                ("c", "config: sync .crypt-env.yaml ⇄ vault (path changes ask first)"),
                ("f", "fill .env + .env.example (master password)"),
                ("s / S", "sync from .env.example / with --global (master password)"),
                ("d", "doctor diagnostics"),
                ("r", "reload"),
                ("q / Ctrl+C", "quit"),
            ];
            let r = centered(area, 76, rows.len() as u16 + 3);
            let body: Vec<Line> = rows
                .iter()
                .map(|(k, d)| {
                    Line::from(vec![
                        Span::styled(format!("{k:<14}"), Style::default().fg(ACCENT)),
                        Span::styled(d.to_string(), Style::default().fg(TX)),
                    ])
                })
                .collect();
            f.render_widget(Clear, r);
            f.render_widget(Paragraph::new(body).block(modal_block(" help ", ACCENT)), r);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::scope::manifest;
    use crate::testing::{env, project, FakeVault};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn app_with(vault: FakeVault, dir: &std::path::Path) -> App {
        App::new(dir.to_path_buf(), Box::new(vault))
    }

    fn password_purpose(app: &App) -> Option<Gated> {
        match &app.modal {
            Modal::Password { purpose, .. } => Some(purpose.clone()),
            _ => None,
        }
    }

    #[test]
    fn expired_session_opens_the_password_modal_for_reload_config_and_init() {
        let dir = tempfile::tempdir().unwrap();
        let vault = FakeVault::new(vec![]);
        vault.alive.set(false);
        let mut app = app_with(vault, dir.path());

        handle_key(&mut app, key('r'));
        assert!(password_purpose(&app) == Some(Gated::Reload));
        app.modal = Modal::None;

        handle_key(&mut app, key('c'));
        assert!(password_purpose(&app) == Some(Gated::Config));
        app.modal = Modal::None;

        handle_key(&mut app, key('i'));
        assert!(matches!(app.modal, Modal::Input { .. }));
        handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(password_purpose(&app), Some(Gated::Init { .. })));
    }

    #[test]
    fn session_required_from_the_client_opens_the_password_modal() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(FakeVault::new(vec![]), dir.path());
        app.fail(CliError::SessionRequired, Gated::Fill);
        assert!(password_purpose(&app) == Some(Gated::Fill));
        app.fail(CliError::VaultLocked, Gated::Fill);
        assert!(matches!(app.modal, Modal::Report { .. }));
    }

    #[test]
    fn ctrl_c_and_q_quit_and_other_ctrl_keys_are_not_plain_letters() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(FakeVault::new(vec![]), dir.path());

        // Ctrl+C must not run `config` (it is not a plain `c`).
        handle_key(&mut app, ctrl('r'));
        handle_key(&mut app, ctrl('f'));
        assert!(matches!(app.modal, Modal::None));
        assert!(!app.quit);

        handle_key(&mut app, ctrl('c'));
        assert!(app.quit);
        assert!(matches!(app.modal, Modal::None));

        let mut app = app_with(FakeVault::new(vec![]), dir.path());
        app.modal = Modal::Password { purpose: Gated::Fill, input: Zeroizing::new("secret".to_string()), error: None };
        handle_key(&mut app, ctrl('c'));
        assert!(app.quit, "Ctrl+C exits even from a modal");

        let mut app = app_with(FakeVault::new(vec![]), dir.path());
        handle_key(&mut app, key('q'));
        assert!(app.quit);
    }

    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }

    fn password_input(app: &App) -> &str {
        match &app.modal {
            Modal::Password { input, .. } => input.as_str(),
            _ => panic!("password modal expected"),
        }
    }

    fn password_app(purpose: Gated, dir: &std::path::Path) -> App {
        let mut app = app_with(FakeVault::new(vec![]), dir);
        app.modal = Modal::Password { purpose, input: new_password_input(), error: None };
        app
    }

    #[test]
    fn password_input_never_reallocates_and_refuses_input_past_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = password_app(Gated::Fill, dir.path());
        let before = match &app.modal {
            Modal::Password { input, .. } => (input.as_ptr(), input.capacity()),
            _ => unreachable!(),
        };
        for _ in 0..(PASSWORD_CAP + 50) {
            handle_key(&mut app, key('a'));
        }
        assert_eq!(password_input(&app).len(), PASSWORD_CAP);
        match &app.modal {
            Modal::Password { input, .. } => {
                assert_eq!((input.as_ptr(), input.capacity()), before, "buffer must not move or grow")
            }
            _ => unreachable!(),
        }
        // A multi-byte char that would cross the cap is refused whole.
        handle_key(&mut app, key('é'));
        assert_eq!(password_input(&app).len(), PASSWORD_CAP);
        handle_key(&mut app, KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(password_input(&app).len(), PASSWORD_CAP - 1);
    }

    #[test]
    fn esc_at_login_wipes_the_typed_password_and_quits() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = password_app(Gated::Login, dir.path());
        for c in "hunter2".chars() {
            handle_key(&mut app, key(c));
        }
        assert_eq!(password_input(&app), "hunter2");
        handle_key(&mut app, esc());
        assert!(app.quit);
        assert_eq!(password_input(&app), "", "typed characters are wiped before exit");
    }

    #[test]
    fn ctrl_c_wipes_the_typed_password() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = password_app(Gated::Fill, dir.path());
        handle_key(&mut app, key('x'));
        handle_key(&mut app, ctrl('c'));
        assert!(app.quit);
        assert_eq!(password_input(&app), "");
    }

    #[test]
    fn esc_on_a_gated_prompt_closes_it_and_enter_moves_the_password_out() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = password_app(Gated::Fill, dir.path());
        handle_key(&mut app, key('x'));
        handle_key(&mut app, esc());
        assert!(matches!(app.modal, Modal::None));

        let mut app = password_app(Gated::Fill, dir.path());
        app.defer = true; // keep the submission queued instead of running it
        handle_key(&mut app, key('p'));
        handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(password_input(&app), "", "the modal no longer holds the submitted password");
        assert!(matches!(&app.pending, Some(Pending::Submit(_, pw)) if pw.as_str() == "p"));
    }

    /// A workspace whose manifest adds `apps/web/.env`; the vault project is
    /// bound to the directory, so only the path diff needs consent.
    fn config_fixture() -> (tempfile::TempDir, FakeVault) {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "project:\n  name: app\n  environments:\n    - {name: default, isDefault: true, paths: ['.env', 'apps/web/.env']}\n";
        std::fs::write(dir.path().join(manifest::FILE_NAME), yaml).unwrap();
        let host = crate::paths::to_host(dir.path());
        let vault = FakeVault::new(vec![project("app", Some(&host), vec![env(1, "default", true, &[".env"])])]);
        (dir, vault)
    }

    #[test]
    fn config_path_change_opens_a_confirm_modal_and_applies_only_on_y() {
        let (dir, vault) = config_fixture();
        let mut app = app_with(vault, dir.path());

        handle_key(&mut app, key('c'));
        match &app.modal {
            Modal::Confirm { lines, then, .. } => {
                assert!(lines.iter().any(|l| l.contains("apps/web/.env")), "{lines:?}");
                assert!(*then == Gated::ConfigPush);
            }
            _ => panic!("expected the confirm modal"),
        }
        assert!(app.api.session_alive().is_ok());

        // `n` cancels: nothing written.
        handle_key(&mut app, key('n'));
        assert!(matches!(app.modal, Modal::None));

        handle_key(&mut app, key('c'));
        handle_key(&mut app, key('y'));
        assert!(matches!(app.modal, Modal::Report { .. }), "applied and reported");
    }

    #[test]
    fn config_confirm_is_gated_again_when_the_session_lapsed() {
        let (dir, vault) = config_fixture();
        let alive = vault.alive.clone();
        let mut app = app_with(vault, dir.path());
        handle_key(&mut app, key('c'));
        assert!(matches!(app.modal, Modal::Confirm { .. }));

        // The session expires while the diff is on screen: `y` must ask for
        // the password rather than prompting on the terminal.
        alive.set(false);
        handle_key(&mut app, key('y'));
        assert!(password_purpose(&app) == Some(Gated::ConfigPush));
    }

    #[test]
    fn config_root_mismatch_is_an_error_without_relink_offer() {
        let (dir, _) = config_fixture();
        let vault = FakeVault::new(vec![project("app", Some("/home/u/elsewhere"), vec![env(1, "default", true, &[".env"])])]);
        let mut app = app_with(vault, dir.path());
        handle_key(&mut app, key('c'));
        match &app.modal {
            Modal::Report { title, lines } => {
                assert_eq!(title, "Error");
                assert!(lines[0].0.contains("/home/u/elsewhere"), "{:?}", lines[0].0);
            }
            _ => panic!("expected an error report"),
        }
    }

    #[test]
    fn idle_redraws_do_not_look_up_the_workspace_or_recompile_the_filter() {
        let (dir, vault) = config_fixture();
        let mut app = app_with(vault, dir.path());
        app.reload().unwrap();
        let lookups = app.config_lookups.get();
        app.filter = "DEF.*".into();
        let builds = app.matcher_builds.get();

        // What an idle tick does: render-time lookups and row filtering.
        for _ in 0..50 {
            let _ = app.config_path();
            let _ = app.rows();
            let _ = app.rows();
        }
        assert_eq!(app.config_lookups.get(), lookups, "no find_config on idle ticks");
        assert_eq!(app.matcher_builds.get(), builds + 1, "filter compiled once per change");

        // `r` re-resolves the workspace exactly once.
        handle_key(&mut app, key('r'));
        assert_eq!(app.config_lookups.get(), lookups + 1);
        app.filter = "other".into();
        let _ = app.rows();
        assert_eq!(app.matcher_builds.get(), builds + 2);
    }

    #[test]
    fn blocking_actions_are_queued_with_a_working_status_when_deferred() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(FakeVault::new(vec![]), dir.path());
        app.defer = true;
        handle_key(&mut app, key('r'));
        assert!(app.pending.is_some(), "not run inline");
        assert_eq!(app.status.as_ref().map(|s| s.0.as_str()), Some(WORKING));
        let p = app.pending.take().unwrap();
        run_pending(&mut app, p);
        assert_eq!(app.status.as_ref().map(|s| s.0.as_str()), Some("Reloaded"));
    }

    #[test]
    fn terminal_is_restored_when_setup_fails_after_raw_mode() {
        let restored = Cell::new(0);
        let r: Result<(TerminalGuard<_>, ()), CliError> = open_terminal(
            || Ok(()),
            || Err(CliError::Api("Terminal::new failed".into())),
            || restored.set(restored.get() + 1),
        );
        assert!(r.is_err());
        assert_eq!(restored.get(), 1, "guard dropped on the error path");

        // Raw mode itself failing means nothing to restore.
        let r: Result<(TerminalGuard<_>, ()), CliError> =
            open_terminal(|| Err(io::Error::other("no tty")), || Ok(()), || restored.set(restored.get() + 1));
        assert!(r.is_err());
        assert_eq!(restored.get(), 1);

        // Success: restored when the guard is dropped.
        let (guard, ()) = open_terminal(|| Ok(()), || Ok(()), || restored.set(restored.get() + 1)).unwrap();
        assert_eq!(restored.get(), 1);
        drop(guard);
        assert_eq!(restored.get(), 2);
    }
}
