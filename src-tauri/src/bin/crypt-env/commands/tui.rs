use clap::Args;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io::{self, Stdout};
use std::time::Duration;

use crate::client::{self, CliError, ItemSummary};
use crate::commands::scope::{self, ResolvedScope};

// ─── Palette ──────────────────────────────────────────────────────────────────

const BG: Color = Color::Rgb(18, 18, 22);
const SURFACE: Color = Color::Rgb(24, 24, 30);
const RAISED: Color = Color::Rgb(30, 30, 38);
const ACCENT: Color = Color::Rgb(0, 200, 150);
const TX: Color = Color::Rgb(220, 220, 225);
const TX2: Color = Color::Rgb(150, 150, 160);
const TX3: Color = Color::Rgb(90, 90, 100);
const DANGER: Color = Color::Rgb(240, 80, 80);

fn type_color(t: &str) -> Color {
    match t {
        "secret" => Color::Rgb(100, 160, 255),
        "credential" => Color::Rgb(255, 200, 60),
        "note" => Color::Rgb(160, 160, 170),
        "link" => Color::Rgb(80, 200, 220),
        "command" => Color::Rgb(200, 100, 255),
        _ => TX2,
    }
}

// ─── App state ────────────────────────────────────────────────────────────────

#[derive(PartialEq)]
enum Screen {
    Unlock,
    Main,
    Detail,
    Help,
    Confirm,
}

struct App {
    screen: Screen,
    // Unlock screen
    password: String,
    pw_error: Option<String>,
    // Main screen
    items: Vec<ItemSummary>,
    list_state: ListState,
    search: String,
    search_mode: bool,
    filtered: Vec<usize>,
    // Detail screen
    revealed_value: Option<String>,
    detail_scroll: u16,
    // Confirm screen
    confirm_msg: String,
    confirm_action: ConfirmAction,
    // Status
    status: Option<(String, bool)>, // (msg, is_error)
    should_quit: bool,
    token: Option<String>,
    // Project/environment scope (resolved once authenticated — see
    // `resolve_and_load`)
    project_flag: Option<String>,
    env_flag: Option<String>,
    scope: Option<ResolvedScope>,
}

#[derive(Clone)]
enum ConfirmAction {
    DeleteItem(i64),
    None,
}

impl App {
    fn new(project_flag: Option<String>, env_flag: Option<String>) -> Self {
        let token = client::read_token();
        let screen = if token.is_some() { Screen::Main } else { Screen::Unlock };
        App {
            screen,
            password: String::new(),
            pw_error: None,
            items: Vec::new(),
            list_state: ListState::default(),
            search: String::new(),
            search_mode: false,
            filtered: Vec::new(),
            revealed_value: None,
            detail_scroll: 0,
            confirm_msg: String::new(),
            confirm_action: ConfirmAction::None,
            status: None,
            should_quit: false,
            token,
            project_flag,
            env_flag,
            scope: None,
        }
    }

    /// Resolves project/environment scope once (memoized in `self.scope`) —
    /// called after authentication succeeds, since resolution may issue
    /// authenticated requests (GET/POST /projects).
    fn resolve_scope(&mut self) -> Result<(), CliError> {
        if self.scope.is_none() {
            self.scope = Some(scope::resolve(self.project_flag.as_deref(), self.env_flag.as_deref(), false)?);
        }
        Ok(())
    }

    /// Loads items for the resolved scope. Panics-free: returns the error
    /// for the caller to surface as pw_error / status.
    fn load_items(&mut self) -> Result<Vec<ItemSummary>, CliError> {
        self.resolve_scope()?;
        let query = self.scope.as_ref().expect("scope resolved above").to_query_string();
        client::api_list_items(&query)
    }

    /// "project / environment" for display in the UI chrome, before scope is
    /// resolved (e.g. still on the Unlock screen).
    fn scope_label(&self) -> String {
        match &self.scope {
            Some(s) => format!("{} / {}", s.project, s.environment),
            None => "—".to_string(),
        }
    }

    fn visible_items(&self) -> Vec<&ItemSummary> {
        if self.filtered.is_empty() && self.search.is_empty() {
            self.items.iter().collect()
        } else {
            self.filtered.iter().filter_map(|i| self.items.get(*i)).collect()
        }
    }

    fn selected_item(&self) -> Option<&ItemSummary> {
        let sel = self.list_state.selected()?;
        let visible = self.visible_items();
        visible.get(sel).copied()
    }

    fn apply_search(&mut self) {
        if self.search.is_empty() {
            self.filtered.clear();
            return;
        }
        let q = self.search.to_lowercase();
        self.filtered = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                item.name.as_deref().map(|n| n.to_lowercase().contains(&q)).unwrap_or(false)
                    || item.title.as_deref().map(|t| t.to_lowercase().contains(&q)).unwrap_or(false)
                    || item.item_type.to_lowercase().contains(&q)
                    || item.categories.iter().any(|c| c.to_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect();
        // Reset selection if out of bounds
        let len = self.filtered.len();
        if len == 0 {
            self.list_state.select(None);
        } else if self.list_state.selected().map(|s| s >= len).unwrap_or(true) {
            self.list_state.select(Some(0));
        }
    }

    fn set_status(&mut self, msg: impl Into<String>, is_error: bool) {
        self.status = Some((msg.into(), is_error));
    }
}

// ─── CLI entry point ──────────────────────────────────────────────────────────

#[derive(Args)]
pub struct TuiArgs {
    /// Project to browse (defaults to crypt-env.json or the cwd folder name)
    #[arg(long)]
    pub project: Option<String>,

    /// Environment to browse (defaults to crypt-env.json or the project's default environment)
    #[arg(long = "env")]
    pub env: Option<String>,
}

pub fn run(args: TuiArgs) -> Result<(), CliError> {
    enable_raw_mode().map_err(|e| CliError::Io(e))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|e| CliError::Io(e))?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| CliError::Api(e.to_string()))?;

    // Ensure terminal is restored even on panic
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));

    let mut app = App::new(args.project, args.env);

    // Try to resolve scope and load items if already authenticated
    if app.screen == Screen::Main {
        match app.load_items() {
            Ok(items) => {
                let len = items.len();
                app.items = items;
                if len > 0 {
                    app.list_state.select(Some(0));
                }
            }
            Err(_) => {
                // Token may be stale, or scope resolution failed — fall
                // back to unlock rather than showing a broken item list.
                client::clear_token();
                app.token = None;
                app.scope = None;
                app.screen = Screen::Unlock;
            }
        }
    }

    let result = run_loop(&mut terminal, &mut app);

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> Result<(), CliError> {
    loop {
        terminal.draw(|f| render(f, app)).map_err(|e| CliError::Api(e.to_string()))?;

        if event::poll(Duration::from_millis(80)).map_err(|e| CliError::Io(e))? {
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                handle_key(app, key.code, key.modifiers);
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

// ─── Input handling ───────────────────────────────────────────────────────────

fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    match app.screen {
        Screen::Unlock => handle_unlock(app, code),
        Screen::Main => handle_main(app, code, mods),
        Screen::Detail => handle_detail(app, code),
        Screen::Help => {
            app.screen = Screen::Main;
        }
        Screen::Confirm => handle_confirm(app, code),
    }
}

fn handle_unlock(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char(c) => {
            app.password.push(c);
            app.pw_error = None;
        }
        KeyCode::Backspace => {
            app.password.pop();
        }
        KeyCode::Enter => {
            let pw = app.password.clone();
            match client::api_unlock(&pw) {
                Ok(token) => {
                    client::save_token(&token);
                    app.token = Some(token);
                    app.password.clear();
                    app.pw_error = None;
                    // Resolve project/environment scope and load its items
                    match app.load_items() {
                        Ok(items) => {
                            let len = items.len();
                            app.items = items;
                            if len > 0 {
                                app.list_state.select(Some(0));
                            }
                            app.screen = Screen::Main;
                        }
                        Err(e) => {
                            app.pw_error = Some(e.to_string());
                        }
                    }
                }
                Err(e) => {
                    app.pw_error = Some(e.to_string());
                    app.password.clear();
                }
            }
        }
        KeyCode::Esc => {
            app.should_quit = true;
        }
        _ => {}
    }
}

fn handle_main(app: &mut App, code: KeyCode, _mods: KeyModifiers) {
    if app.search_mode {
        match code {
            KeyCode::Esc => {
                app.search_mode = false;
                app.search.clear();
                app.filtered.clear();
                if !app.items.is_empty() {
                    app.list_state.select(Some(0));
                }
            }
            KeyCode::Backspace => {
                app.search.pop();
                app.apply_search();
            }
            KeyCode::Char(c) => {
                app.search.push(c);
                app.apply_search();
            }
            KeyCode::Enter | KeyCode::Down => {
                app.search_mode = false;
                let len = app.visible_items().len();
                if len > 0 && app.list_state.selected().is_none() {
                    app.list_state.select(Some(0));
                }
            }
            _ => {}
        }
        return;
    }

    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('?') => app.screen = Screen::Help,
        KeyCode::Char('/') => {
            app.search_mode = true;
            app.search.clear();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let len = app.visible_items().len();
            if len > 0 {
                let next = match app.list_state.selected() {
                    Some(i) => (i + 1).min(len - 1),
                    None => 0,
                };
                app.list_state.select(Some(next));
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let len = app.visible_items().len();
            if len > 0 {
                let prev = match app.list_state.selected() {
                    Some(i) => i.saturating_sub(1),
                    None => 0,
                };
                app.list_state.select(Some(prev));
            }
        }
        KeyCode::Enter => {
            if app.selected_item().is_some() {
                app.revealed_value = None;
                app.detail_scroll = 0;
                app.screen = Screen::Detail;
            }
        }
        KeyCode::Char('r') => {
            // Refresh (within the already-resolved scope)
            match app.load_items() {
                Ok(items) => {
                    let len = items.len();
                    app.items = items;
                    app.apply_search();
                    if len > 0 && app.list_state.selected().is_none() {
                        app.list_state.select(Some(0));
                    }
                    app.set_status("Refreshed", false);
                }
                Err(e) => app.set_status(e.to_string(), true),
            }
        }
        KeyCode::Char('d') => {
            if let Some(item) = app.selected_item() {
                let id = item.id;
                let name = item.name.clone().or(item.title.clone()).unwrap_or_else(|| format!("#{id}"));
                app.confirm_msg = format!("Delete '{name}'?");
                app.confirm_action = ConfirmAction::DeleteItem(id);
                app.screen = Screen::Confirm;
            }
        }
        _ => {}
    }
}

fn handle_detail(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.revealed_value = None;
            app.screen = Screen::Main;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.detail_scroll = app.detail_scroll.saturating_add(1);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.detail_scroll = app.detail_scroll.saturating_sub(1);
        }
        KeyCode::Char('v') => {
            if let Some(item) = app.selected_item() {
                let id = item.id;
                if let Some(token) = &app.token.clone() {
                    match client::api_reveal(id, token) {
                        Ok(v) => {
                            app.revealed_value = Some(v);
                            app.set_status("Value revealed", false);
                        }
                        Err(e) => app.set_status(e.to_string(), true),
                    }
                }
            }
        }
        KeyCode::Char('c') => {
            if let Some(val) = &app.revealed_value {
                #[cfg(target_os = "windows")]
                {
                    let _ = std::process::Command::new("clip")
                        .stdin(std::process::Stdio::piped())
                        .spawn()
                        .and_then(|mut child| {
                            use std::io::Write;
                            child.stdin.as_mut().unwrap().write_all(val.as_bytes())?;
                            child.wait()
                        });
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let _ = std::process::Command::new("pbcopy")
                        .stdin(std::process::Stdio::piped())
                        .spawn()
                        .or_else(|_| {
                            std::process::Command::new("xclip")
                                .args(["-selection", "clipboard"])
                                .stdin(std::process::Stdio::piped())
                                .spawn()
                        })
                        .and_then(|mut child| {
                            use std::io::Write;
                            child.stdin.as_mut().unwrap().write_all(val.as_bytes())?;
                            child.wait()
                        });
                }
                app.set_status("Copied to clipboard", false);
            }
        }
        _ => {}
    }
}

fn handle_confirm(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('y') | KeyCode::Enter => {
            let action = app.confirm_action.clone();
            match action {
                ConfirmAction::DeleteItem(id) => {
                    let url = format!("{}/items/{}", client::api_base(), id);
                    match client::authenticated_delete(&url) {
                        Ok(_) => {
                            app.items.retain(|i| i.id != id);
                            app.apply_search();
                            let len = app.visible_items().len();
                            if len == 0 {
                                app.list_state.select(None);
                            } else if app.list_state.selected().map(|s| s >= len).unwrap_or(false) {
                                app.list_state.select(Some(len - 1));
                            }
                            app.set_status("Item deleted", false);
                        }
                        Err(e) => app.set_status(e.to_string(), true),
                    }
                }
                ConfirmAction::None => {}
            }
            app.screen = Screen::Main;
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            app.screen = Screen::Main;
        }
        _ => {}
    }
}

// ─── Rendering ────────────────────────────────────────────────────────────────

fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    // Background
    f.render_widget(Block::default().style(Style::default().bg(BG)), area);

    match app.screen {
        Screen::Unlock => render_unlock(f, app, area),
        Screen::Main => render_main(f, app, area),
        Screen::Detail => render_detail(f, app, area),
        Screen::Help => render_help(f, area),
        Screen::Confirm => {
            render_main(f, app, area);
            render_confirm_overlay(f, app, area);
        }
    }
}

fn render_unlock(f: &mut Frame, app: &App, area: Rect) {
    let vertical = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(10),
        Constraint::Fill(1),
    ])
    .split(area);

    let horizontal = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Min(40),
        Constraint::Max(54),
        Constraint::Fill(1),
    ])
    .split(vertical[1]);

    let inner = horizontal[2];

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(" CRYPTENV ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .style(Style::default().bg(SURFACE));

    let inner_area = block.inner(inner);
    f.render_widget(block, inner);

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner_area);

    let label = Paragraph::new("  Master Password:")
        .style(Style::default().fg(TX2));
    f.render_widget(label, rows[1]);

    let pw_display = "•".repeat(app.password.len());
    let pw_style = if app.pw_error.is_some() {
        Style::default().fg(DANGER)
    } else {
        Style::default().fg(TX)
    };
    let pw = Paragraph::new(format!("  {}_", pw_display)).style(pw_style);
    f.render_widget(pw, rows[2]);

    if let Some(err) = &app.pw_error {
        let err_text = Paragraph::new(format!("  ✗ {}", err))
            .style(Style::default().fg(DANGER));
        f.render_widget(err_text, rows[3]);
    } else {
        let hint = Paragraph::new("  [Enter] unlock · [Esc] quit")
            .style(Style::default().fg(TX3));
        f.render_widget(hint, rows[4]);
    }
}

fn render_main(f: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .split(area);

    // Top bar
    render_topbar(f, app, rows[0]);

    // Split: list left, detail right
    let cols = Layout::horizontal([Constraint::Percentage(55), Constraint::Fill(1)]).split(rows[1]);

    render_item_list(f, app, cols[0]);
    render_item_preview(f, app, cols[1]);

    // Bottom status bar
    render_statusbar(f, app, rows[2]);
}

fn render_topbar(f: &mut Frame, app: &App, area: Rect) {
    let search_text = if app.search_mode {
        format!(" / {} ", app.search)
    } else if !app.search.is_empty() {
        format!(" /{} ", app.search)
    } else {
        " CRYPTENV ".to_string()
    };
    let total = app.items.len();
    let shown = if app.search.is_empty() { total } else { app.filtered.len() };
    let count = format!(" {}/{} ", shown, total);
    let scope_text = format!(" [{}] ", app.scope_label());

    let bar = Paragraph::new(Line::from(vec![
        Span::styled(search_text, Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled(scope_text, Style::default().fg(TX2)),
        Span::styled(count, Style::default().fg(TX3)),
    ]))
    .style(Style::default().bg(RAISED));
    f.render_widget(bar, area);
}

fn render_item_list(f: &mut Frame, app: &App, area: Rect) {
    let visible = app.visible_items();
    let items: Vec<ListItem> = visible
        .iter()
        .map(|item| {
            let name = item.name.as_deref().or(item.title.as_deref()).unwrap_or("(unnamed)");
            let type_badge = format!("{:<10}", item.item_type.to_uppercase());
            let cats = if item.categories.is_empty() {
                String::new()
            } else {
                format!(" [{}]", item.categories.join(", "))
            };
            ListItem::new(Line::from(vec![
                Span::styled(type_badge, Style::default().fg(type_color(&item.item_type))),
                Span::styled(name, Style::default().fg(TX)),
                Span::styled(cats, Style::default().fg(TX3)),
            ]))
        })
        .collect();

    let selected_style = Style::default()
        .bg(Color::Rgb(30, 35, 45))
        .fg(ACCENT)
        .add_modifier(Modifier::BOLD);

    let list = List::new(items)
        .block(
            Block::default()
                .title(Span::styled(" Vault ", Style::default().fg(TX2)))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Rgb(50, 50, 60)))
                .style(Style::default().bg(BG)),
        )
        .highlight_style(selected_style)
        .highlight_symbol("▶ ");

    let mut state = app.list_state.clone();
    f.render_stateful_widget(list, area, &mut state);
}

fn render_item_preview(f: &mut Frame, app: &App, area: Rect) {
    let content = if let Some(item) = app.selected_item() {
        let mut lines = vec![
            Line::from(vec![
                Span::styled("  Type:  ", Style::default().fg(TX3)),
                Span::styled(item.item_type.to_uppercase(), Style::default().fg(type_color(&item.item_type)).add_modifier(Modifier::BOLD)),
            ]),
        ];
        if let Some(name) = &item.name {
            lines.push(Line::from(vec![
                Span::styled("  Name:  ", Style::default().fg(TX3)),
                Span::styled(name.clone(), Style::default().fg(TX)),
            ]));
        }
        if let Some(title) = &item.title {
            lines.push(Line::from(vec![
                Span::styled("  Title: ", Style::default().fg(TX3)),
                Span::styled(title.clone(), Style::default().fg(TX)),
            ]));
        }
        if !item.categories.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  Tags:  ", Style::default().fg(TX3)),
                Span::styled(item.categories.join(", "), Style::default().fg(ACCENT)),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  [Enter] detail   [v] reveal   [d] delete", Style::default().fg(TX3)),
        ]));
        lines
    } else {
        vec![Line::from(Span::styled(
            "  No item selected",
            Style::default().fg(TX3),
        ))]
    };

    let block = Block::default()
        .title(Span::styled(" Preview ", Style::default().fg(TX2)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Rgb(50, 50, 60)))
        .style(Style::default().bg(BG));

    let paragraph = Paragraph::new(content).block(block).wrap(Wrap { trim: false });
    f.render_widget(paragraph, area);
}

fn render_statusbar(f: &mut Frame, app: &App, area: Rect) {
    let msg = if let Some((msg, is_err)) = &app.status {
        let style = if *is_err {
            Style::default().fg(DANGER)
        } else {
            Style::default().fg(ACCENT)
        };
        Span::styled(format!(" {}", msg), style)
    } else {
        Span::styled(" [↑↓/jk] nav  [/] search  [Enter] detail  [v] reveal  [d] delete  [r] refresh  [?] help  [q] quit", Style::default().fg(TX3))
    };

    let bar = Paragraph::new(Line::from(vec![msg]))
        .style(Style::default().bg(RAISED));
    f.render_widget(bar, area);
}

fn render_detail(f: &mut Frame, app: &App, area: Rect) {
    if let Some(item) = app.selected_item() {
        let mut lines = vec![
            Line::from(vec![
                Span::styled("  ID:    ", Style::default().fg(TX3)),
                Span::styled(item.id.to_string(), Style::default().fg(TX2)),
            ]),
            Line::from(vec![
                Span::styled("  Type:  ", Style::default().fg(TX3)),
                Span::styled(item.item_type.to_uppercase(), Style::default().fg(type_color(&item.item_type)).add_modifier(Modifier::BOLD)),
            ]),
        ];
        if let Some(name) = &item.name {
            lines.push(Line::from(vec![
                Span::styled("  Name:  ", Style::default().fg(TX3)),
                Span::styled(name.clone(), Style::default().fg(TX)),
            ]));
        }
        if let Some(title) = &item.title {
            lines.push(Line::from(vec![
                Span::styled("  Title: ", Style::default().fg(TX3)),
                Span::styled(title.clone(), Style::default().fg(TX)),
            ]));
        }
        if !item.categories.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  Tags:  ", Style::default().fg(TX3)),
                Span::styled(item.categories.join(", "), Style::default().fg(ACCENT)),
            ]));
        }
        lines.push(Line::from(""));
        if let Some(val) = &app.revealed_value {
            lines.push(Line::from(vec![
                Span::styled("  Value: ", Style::default().fg(TX3)),
                Span::styled(val.clone(), Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  [c] copy to clipboard", Style::default().fg(TX3))));
        } else {
            lines.push(Line::from(vec![
                Span::styled("  Value: ", Style::default().fg(TX3)),
                Span::styled("••••••••••••", Style::default().fg(TX3)),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  [v] reveal value", Style::default().fg(TX3))));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  [Esc/q] back", Style::default().fg(TX3))));

        let block = Block::default()
            .title(Span::styled(" Item Detail ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(ACCENT))
            .style(Style::default().bg(SURFACE));

        let para = Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((app.detail_scroll, 0));
        f.render_widget(para, area);
    }
}

fn render_help(f: &mut Frame, area: Rect) {
    let rows = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(20),
        Constraint::Fill(1),
    ])
    .split(area);
    let cols = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(52),
        Constraint::Fill(1),
    ])
    .split(rows[1]);

    let lines = vec![
        Line::from(Span::styled("  Navigation", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![Span::styled("  ↑↓ / j k   ", Style::default().fg(TX2)), Span::styled("Move selection", Style::default().fg(TX))]),
        Line::from(vec![Span::styled("  /          ", Style::default().fg(TX2)), Span::styled("Start fuzzy search", Style::default().fg(TX))]),
        Line::from(vec![Span::styled("  Esc        ", Style::default().fg(TX2)), Span::styled("Clear search / go back", Style::default().fg(TX))]),
        Line::from(vec![Span::styled("  Enter      ", Style::default().fg(TX2)), Span::styled("Open item detail", Style::default().fg(TX))]),
        Line::from(""),
        Line::from(Span::styled("  Item Actions", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![Span::styled("  v          ", Style::default().fg(TX2)), Span::styled("Reveal secret value", Style::default().fg(TX))]),
        Line::from(vec![Span::styled("  c          ", Style::default().fg(TX2)), Span::styled("Copy revealed value to clipboard", Style::default().fg(TX))]),
        Line::from(vec![Span::styled("  d          ", Style::default().fg(TX2)), Span::styled("Delete item (confirm required)", Style::default().fg(TX))]),
        Line::from(vec![Span::styled("  r          ", Style::default().fg(TX2)), Span::styled("Refresh vault items", Style::default().fg(TX))]),
        Line::from(""),
        Line::from(Span::styled("  Global", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![Span::styled("  ?          ", Style::default().fg(TX2)), Span::styled("Toggle this help", Style::default().fg(TX))]),
        Line::from(vec![Span::styled("  q / Esc    ", Style::default().fg(TX2)), Span::styled("Quit TUI", Style::default().fg(TX))]),
    ];

    let block = Block::default()
        .title(Span::styled(" Help ", Style::default().fg(ACCENT)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT))
        .style(Style::default().bg(SURFACE));

    let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, cols[1]);

    let dismiss = Paragraph::new("  Press any key to close")
        .style(Style::default().fg(TX3).bg(RAISED));
    f.render_widget(
        dismiss,
        Rect {
            x: cols[1].x,
            y: cols[1].y + cols[1].height,
            width: cols[1].width,
            height: 1,
        },
    );
}

fn render_confirm_overlay(f: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(5),
        Constraint::Fill(1),
    ])
    .split(area);
    let cols = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(46),
        Constraint::Fill(1),
    ])
    .split(rows[1]);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", app.confirm_msg),
            Style::default().fg(TX),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [y] confirm   ", Style::default().fg(DANGER).add_modifier(Modifier::BOLD)),
            Span::styled("[n/Esc] cancel", Style::default().fg(TX3)),
        ]),
    ];

    let block = Block::default()
        .title(Span::styled(" Confirm ", Style::default().fg(DANGER)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DANGER))
        .style(Style::default().bg(SURFACE));

    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, cols[1]);
}
