//! Skills screen: installed skills, MCP servers, quick start.

use crate::tui::theme;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Padding, Paragraph};
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct SkillInfo {
    pub name: String,
    pub runtime: String,
    pub source: String,
    pub description: String,
}

#[derive(Clone, Default)]
pub struct McpServerInfo {
    pub name: String,
    pub connected: bool,
    pub tool_count: usize,
}

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SkillsSub {
    Installed,
    QuickStart,
    Mcp,
}

pub struct SkillsState {
    pub sub: SkillsSub,
    pub installed: Vec<SkillInfo>,
    pub mcp_servers: Vec<McpServerInfo>,
    pub installed_list: ListState,
    pub mcp_list: ListState,
    pub quickstart_list: ListState,
    pub loading: bool,
    pub tick: usize,
    pub confirm_uninstall: bool,
    pub status_msg: String,
}

pub enum SkillsAction {
    Continue,
    RefreshInstalled,
    UninstallSkill(String),
    RefreshMcp,
}

impl SkillsState {
    pub fn new() -> Self {
        Self {
            sub: SkillsSub::Installed,
            installed: Vec::new(),
            mcp_servers: Vec::new(),
            installed_list: ListState::default(),
            mcp_list: ListState::default(),
            quickstart_list: ListState::default(),
            loading: false,
            tick: 0,
            confirm_uninstall: false,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SkillsAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return SkillsAction::Continue;
        }

        // Tab switching within Skills (1/2/3)
        match key.code {
            KeyCode::Char('1') => {
                self.sub = SkillsSub::Installed;
                return SkillsAction::RefreshInstalled;
            }
            KeyCode::Char('2') => {
                self.sub = SkillsSub::QuickStart;
            }
            KeyCode::Char('3') => {
                self.sub = SkillsSub::Mcp;
                return SkillsAction::RefreshMcp;
            }
            _ => {}
        }

        match self.sub {
            SkillsSub::Installed => self.handle_installed(key),
            SkillsSub::QuickStart => SkillsAction::Continue,
            SkillsSub::Mcp => self.handle_mcp(key),
        }
    }

    fn handle_installed(&mut self, key: KeyEvent) -> SkillsAction {
        if self.confirm_uninstall {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.confirm_uninstall = false;
                    if let Some(sel) = self.installed_list.selected() {
                        if sel < self.installed.len() {
                            return SkillsAction::UninstallSkill(self.installed[sel].name.clone());
                        }
                    }
                }
                _ => self.confirm_uninstall = false,
            }
            return SkillsAction::Continue;
        }

        let total = self.installed.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if total > 0 {
                    let i = self.installed_list.selected().unwrap_or(0);
                    let next = if i == 0 { total - 1 } else { i - 1 };
                    self.installed_list.select(Some(next));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if total > 0 {
                    let i = self.installed_list.selected().unwrap_or(0);
                    let next = (i + 1) % total;
                    self.installed_list.select(Some(next));
                }
            }
            KeyCode::Char('u') => {
                if self.installed_list.selected().is_some() {
                    self.confirm_uninstall = true;
                }
            }
            KeyCode::Char('r') => return SkillsAction::RefreshInstalled,
            _ => {}
        }
        SkillsAction::Continue
    }

    fn handle_mcp(&mut self, key: KeyEvent) -> SkillsAction {
        let total = self.mcp_servers.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if total > 0 {
                    let i = self.mcp_list.selected().unwrap_or(0);
                    let next = if i == 0 { total - 1 } else { i - 1 };
                    self.mcp_list.select(Some(next));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if total > 0 {
                    let i = self.mcp_list.selected().unwrap_or(0);
                    let next = (i + 1) % total;
                    self.mcp_list.select(Some(next));
                }
            }
            KeyCode::Char('r') => return SkillsAction::RefreshMcp,
            _ => {}
        }
        SkillsAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut SkillsState) {
    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            " Skills ",
            theme::title_style(),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT))
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(1), // sub-tab bar
        Constraint::Length(1), // separator
        Constraint::Min(3),    // content
    ])
    .split(inner);

    // Sub-tab bar
    draw_sub_tabs(f, chunks[0], state.sub);

    let sep = "\u{2500}".repeat(chunks[1].width as usize);
    f.render_widget(
        Paragraph::new(Span::styled(sep, theme::dim_style())),
        chunks[1],
    );

    match state.sub {
        SkillsSub::Installed => draw_installed(f, chunks[2], state),
        SkillsSub::QuickStart => draw_quickstart(f, chunks[2], state),
        SkillsSub::Mcp => draw_mcp(f, chunks[2], state),
    }
}

fn draw_sub_tabs(f: &mut Frame, area: Rect, active: SkillsSub) {
    let tabs = [
        (SkillsSub::Installed, "1 Installed"),
        (SkillsSub::QuickStart, "2 Quick Start"),
        (SkillsSub::Mcp, "3 MCP Servers"),
    ];
    let mut spans = vec![Span::raw("  ")];
    for (sub, label) in &tabs {
        let style = if *sub == active {
            theme::tab_active()
        } else {
            theme::tab_inactive()
        };
        spans.push(Span::styled(format!(" {label} "), style));
        spans.push(Span::raw(" "));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_installed(f: &mut Frame, area: Rect, state: &mut SkillsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(
                "  {:<20} {:<8} {:<12} {}",
                "Name", "Runtime", "Source", "Description"
            ),
            theme::table_header(),
        )])),
        chunks[0],
    );

    if state.loading {
        let spinner = theme::SPINNER_FRAMES[state.tick % theme::SPINNER_FRAMES.len()];
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("  {spinner} "), Style::default().fg(theme::CYAN)),
                Span::styled("Loading skills\u{2026}", theme::dim_style()),
            ])),
            chunks[1],
        );
    } else if state.installed.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "  No skills installed. Press [2] for Quick Start templates.",
                theme::dim_style(),
            )),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .installed
            .iter()
            .map(|s| {
                let runtime_style = match s.runtime.as_str() {
                    "python" | "py" => Style::default().fg(theme::BLUE),
                    "node" | "js" => Style::default().fg(theme::YELLOW),
                    "wasm" => Style::default().fg(theme::PURPLE),
                    _ => Style::default().fg(theme::GREEN),
                };
                let runtime_badge = match s.runtime.as_str() {
                    "python" | "py" => "PY",
                    "node" | "js" => "JS",
                    "wasm" => "WASM",
                    "prompt" => "PROMPT",
                    _ => &s.runtime,
                };
                let source_style = match s.source.as_str() {
                    "builtin" | "built-in" => Style::default().fg(theme::GREEN),
                    _ => theme::dim_style(),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<20}", truncate(&s.name, 19)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(format!(" {:<8}", runtime_badge), runtime_style),
                    Span::styled(format!(" {:<12}", &s.source), source_style),
                    Span::styled(
                        format!(" {}", truncate(&s.description, 30)),
                        theme::dim_style(),
                    ),
                ]))
            })
            .collect();

        let list = List::new(items)
            .highlight_style(theme::selected_style())
            .highlight_symbol("> ");
        f.render_stateful_widget(list, chunks[1], &mut state.installed_list);
    }

    if state.confirm_uninstall {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                "  Uninstall this skill? [y] Yes  [any] Cancel",
                Style::default().fg(theme::YELLOW),
            )])),
            chunks[2],
        );
    } else if !state.status_msg.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!("  {}", state.status_msg),
                Style::default().fg(theme::GREEN),
            )])),
            chunks[2],
        );
    } else {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                "  [\u{2191}\u{2193}] Navigate  [u] Uninstall  [r] Refresh",
                theme::hint_style(),
            )])),
            chunks[2],
        );
    }
}

fn draw_quickstart(f: &mut Frame, area: Rect, state: &mut SkillsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // content
        Constraint::Length(1), // hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  Quick Start Templates",
            theme::title_style(),
        )])),
        chunks[0],
    );

    let items: Vec<ListItem> = [
        ("code-review-guide", "Adds code review best practices and checklist to agent context."),
        ("writing-style", "Configurable writing style guide for content generation."),
        ("api-design", "REST API design patterns and conventions."),
        ("security-checklist", "OWASP-aligned security review checklist."),
    ]
    .iter()
    .map(|(name, desc)| {
        ListItem::new(Line::from(vec![
            Span::styled(
                format!("  {:<24}", name),
                Style::default().fg(theme::CYAN),
            ),
            Span::styled(
                format!(" {}", desc),
                theme::dim_style(),
            ),
        ]))
    })
    .collect();

    let list = List::new(items)
        .highlight_style(theme::selected_style())
        .highlight_symbol("> ");
    f.render_stateful_widget(list, chunks[1], &mut state.quickstart_list);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  Prompt-only skill templates (zero dependencies)",
            theme::hint_style(),
        )])),
        chunks[2],
    );
}

fn draw_mcp(f: &mut Frame, area: Rect, state: &mut SkillsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {:<20} {:<14} {}", "Server", "Status", "Tools"),
            theme::table_header(),
        )])),
        chunks[0],
    );

    if state.loading {
        let spinner = theme::SPINNER_FRAMES[state.tick % theme::SPINNER_FRAMES.len()];
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("  {spinner} "), Style::default().fg(theme::CYAN)),
                Span::styled("Loading MCP servers\u{2026}", theme::dim_style()),
            ])),
            chunks[1],
        );
    } else if state.mcp_servers.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "  No MCP servers configured.",
                theme::dim_style(),
            )),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .mcp_servers
            .iter()
            .map(|s| {
                let (badge, style) = if s.connected {
                    ("\u{2714} Connected", Style::default().fg(theme::GREEN))
                } else {
                    ("\u{2718} Disconnected", Style::default().fg(theme::RED))
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("  {:<20}", truncate(&s.name, 19)),
                        Style::default().fg(theme::CYAN),
                    ),
                    Span::styled(format!(" {:<14}", badge), style),
                    Span::styled(format!(" {}", s.tool_count), theme::dim_style()),
                ]))
            })
            .collect();

        let list = List::new(items)
            .highlight_style(theme::selected_style())
            .highlight_symbol("> ");
        f.render_stateful_widget(list, chunks[1], &mut state.mcp_list);
    }

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  [\u{2191}\u{2193}] Navigate  [r] Refresh",
            theme::hint_style(),
        )])),
        chunks[2],
    );
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!(
            "{}\u{2026}",
            opencarrier_types::truncate_str(s, max.saturating_sub(1))
        )
    }
}
