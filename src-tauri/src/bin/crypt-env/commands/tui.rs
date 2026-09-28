//! `crypt-env tui` — keyboard-driven terminal UI (ratatui) mirroring the CLI
//! project workflow: browse projects → environments → variables (values
//! masked), `/` filter, password-gated reveal (`v`), and the workspace
//! actions `init`, `config`, `fill`, `sync` and `doctor` on the current
//! directory's `.crypt-env.yaml`.

use clap::Args;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
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
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::time::Duration;
use zeroize::{Zeroize, Zeroizing};

use crate::client::{self, CliError, ItemSummary, Project};
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

/// What a password prompt unlocks once verified.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Gated {
    Login,
    Reveal,
    Fill,
    Sync { global: bool },
}

enum Modal {
    None,
    Password { purpose: Gated, input: String, error: Option<String> },
    Input { title: &'static str, value: String },
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
}

impl App {
    fn new(workspace_dir: PathBuf) -> Self {
        App {
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
        let m = search::Matcher::new(if self.filter.is_empty() { None } else { Some(&self.filter) });
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
        self.projects = client::fetch_projects()?;
        let ws_name = scope::find_config(&self.workspace_dir)
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
                client::list_items(&p.name, &env.name, "only")?
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
}

// ─── Entry / loop ─────────────────────────────────────────────────────────────

pub fn run(_args: TuiArgs) -> Result<(), CliError> {
    let cwd = std::env::current_dir()?;
    enable_raw_mode().map_err(CliError::Io)?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(CliError::Io)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout)).map_err(|e| CliError::Api(e.to_string()))?;

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));

    let mut app = App::new(cwd);
    if !matches!(client::session_alive(), Ok(true)) || app.reload().is_err() {
        app.modal = Modal::Password { purpose: Gated::Login, input: String::new(), error: None };
    }
    let result = run_loop(&mut terminal, &mut app);

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();
    result
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<(), CliError> {
    while !app.quit {
        terminal.draw(|f| render(f, app)).map_err(|e| CliError::Api(e.to_string()))?;
        if event::poll(Duration::from_millis(100)).map_err(CliError::Io)? {
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind == KeyEventKind::Press {
                    handle_key(app, key.code);
                }
            }
        }
    }
    Ok(())
}

// ─── Input ────────────────────────────────────────────────────────────────────

fn handle_key(app: &mut App, code: KeyCode) {
    match &mut app.modal {
        Modal::None => {}
        Modal::Password { input, purpose, .. } => {
            match code {
                KeyCode::Char(c) => input.push(c),
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Esc if *purpose == Gated::Login => app.quit = true,
                KeyCode::Esc => {
                    input.zeroize();
                    app.modal = Modal::None;
                }
                KeyCode::Enter => {
                    let purpose = *purpose;
                    let pw = Zeroizing::new(std::mem::take(input));
                    submit_password(app, purpose, &pw);
                }
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
                    run_init(app, &name);
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
        KeyCode::Char('r') => {
            if let Err(e) = app.reload() {
                app.error(e);
            } else {
                app.status = Some(("Reloaded".into(), false));
            }
        }
        KeyCode::Char('i') => {
            let default = app.workspace_dir.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            app.modal = Modal::Input { title: " init — project name (Enter to confirm) ", value: default };
        }
        KeyCode::Char('c') => run_config(app),
        KeyCode::Char('f') => gate(app, Gated::Fill),
        KeyCode::Char('s') => gate(app, Gated::Sync { global: false }),
        KeyCode::Char('S') => gate(app, Gated::Sync { global: true }),
        KeyCode::Char('d') => {
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
        _ => {}
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
    match client::session_alive() {
        Ok(true) => run_gated(app, purpose),
        Ok(false) => app.modal = Modal::Password { purpose, input: String::new(), error: None },
        Err(e) => app.error(e),
    }
}

fn submit_password(app: &mut App, purpose: Gated, pw: &str) {
    if let Err(e) = client::authenticate(pw) {
        app.modal = Modal::Password { purpose, input: String::new(), error: Some(e.to_string()) };
        return;
    }
    app.modal = Modal::None;
    run_gated(app, purpose);
}

fn run_gated(app: &mut App, purpose: Gated) {
    match purpose {
        Gated::Login => {
            if let Err(e) = app.reload() {
                app.error(e);
            }
        }
        Gated::Reveal => {
            let rows = app.rows();
            let Some(row) = app.var_state.selected().and_then(|i| rows.get(i)) else { return };
            match client::reveal_item(row.item_id) {
                Ok(value) => app.modal = Modal::Reveal { key: row.key.clone(), value },
                Err(e) => app.error(e),
            }
        }
        Gated::Fill => match workspace(app).and_then(|ws| fill::execute(&ws, None)) {
            Ok(r) => {
                let mut lines: Vec<(String, Color)> = r.lines.into_iter().map(|l| (l, TX)).collect();
                lines.extend(r.examples.into_iter().map(|e| (format!("wrote {}", e.display()), TX2)));
                app.report("fill", lines);
            }
            Err(e) => app.error(e),
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
                Err(e) => app.error(e),
            }
        }
    }
}

fn join_or_none(v: &[String]) -> String {
    if v.is_empty() { "none".into() } else { v.join(", ") }
}

/// The cwd workspace, without the legacy-notice `eprintln!` of
/// `scope::workspace` (which would corrupt the alternate screen).
fn workspace(app: &App) -> Result<scope::Workspace, CliError> {
    let path = scope::find_config(&app.workspace_dir).ok_or_else(|| {
        CliError::Config(format!("no {} here — press `i` to init", scope::manifest::FILE_NAME))
    })?;
    scope::load_config(&path)
}

fn run_init(app: &mut App, name: &str) {
    let dir = app.workspace_dir.clone();
    match init::execute(&dir, Some(name), None) {
        Ok(r) => {
            let verb = if r.created { "created" } else { "linked" };
            app.report(
                "init",
                vec![
                    (format!("{verb} project '{}' → {}", r.project, r.target), TX),
                    (format!("wrote {}", r.manifest_path.display()), TX2),
                ],
            );
            let _ = app.reload();
        }
        Err(e) => app.error(e),
    }
}

fn run_config(app: &mut App) {
    let result = workspace(app).and_then(|ws| match ws.manifest.clone() {
        Some(m) => config::execute(&ws.root, &ws.config_path, &m),
        None => Err(CliError::Config("legacy crypt-env.json — press `i` to init".into())),
    });
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
        Err(e) => app.error(e),
    }
}

// ─── Rendering ────────────────────────────────────────────────────────────────

fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(BG)), area);
    let [top, body, bottom] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)]).areas(area);

    let ws = scope::find_config(&app.workspace_dir)
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
                Gated::Reveal => "reveal this value",
                Gated::Fill => "fill .env files",
                Gated::Sync { global: false } => "sync .env.example",
                Gated::Sync { global: true } => "sync --global",
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
                    Line::from(Span::styled(value.as_str().to_string(), Style::default().fg(WARN))),
                    Line::from(Span::styled("any key to hide (value is wiped from memory)", Style::default().fg(TX3))),
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
                ("c", "config: sync .crypt-env.yaml ⇄ vault"),
                ("f", "fill .env + .env.example (master password)"),
                ("s / S", "sync from .env.example / with --global (master password)"),
                ("d", "doctor diagnostics"),
                ("r", "reload"),
                ("q", "quit"),
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
