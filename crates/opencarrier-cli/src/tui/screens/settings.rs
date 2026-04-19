//! Settings screen: Brain three-layer management (Providers, Endpoints, Modalities).

use crate::tui::theme;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Padding, Paragraph};
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct ProviderInfo {
    pub name: String,
    pub configured: bool,
    pub env_var: String,
    pub is_local: bool,
    pub reachable: Option<bool>,
    pub latency_ms: Option<u64>,
}

#[derive(Clone, Default)]
pub struct EndpointInfo {
    pub name: String,
    pub provider: String,
    pub model: String,
    pub format: String,
    #[expect(dead_code)]
    pub base_url: String,
    pub ready: bool,
}

#[derive(Clone, Default)]
pub struct ModalityInfo {
    pub name: String,
    pub primary: String,
    pub fallbacks: Vec<String>,
}

#[derive(Clone)]
pub struct TestResult {
    pub provider: String,
    pub success: bool,
    pub latency_ms: u64,
    pub message: String,
}

/// Form for adding a new endpoint.
#[derive(Clone, Default)]
pub struct EndpointForm {
    pub name: String,
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub format: String,
}

/// Form for adding a new modality.
#[derive(Clone, Default)]
pub struct ModalityForm {
    pub name: String,
    pub primary: String,
    pub fallbacks: String,
}

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsSub {
    Providers,
    Endpoints,
    Modalities,
}

pub struct SettingsState {
    pub sub: SettingsSub,
    pub providers: Vec<ProviderInfo>,
    pub endpoints: Vec<EndpointInfo>,
    pub modalities: Vec<ModalityInfo>,
    pub provider_list: ListState,
    pub endpoint_list: ListState,
    pub modality_list: ListState,
    pub input_buf: String,
    pub input_mode: bool,
    pub editing_provider: Option<String>,
    pub test_result: Option<TestResult>,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
    // Endpoint form
    pub endpoint_form: EndpointForm,
    pub endpoint_form_active: bool,
    pub endpoint_form_field: usize,
    // Modality form
    pub modality_form: ModalityForm,
    pub modality_form_active: bool,
    pub modality_form_field: usize,
}

pub enum SettingsAction {
    Continue,
    RefreshProviders,
    RefreshModels,
    SaveProviderKey { name: String, key: String },
    DeleteProviderKey(String),
    TestProvider(String),
    AddEndpoint { name: String, provider: String, model: String, base_url: String, format: String },
    DeleteEndpoint(String),
    AddModality { name: String, primary: String, fallbacks: Vec<String> },
    DeleteModality(String),
    SetDefaultModality(String),
}

impl SettingsState {
    pub fn new() -> Self {
        Self {
            sub: SettingsSub::Providers,
            providers: Vec::new(),
            endpoints: Vec::new(),
            modalities: Vec::new(),
            provider_list: ListState::default(),
            endpoint_list: ListState::default(),
            modality_list: ListState::default(),
            input_buf: String::new(),
            input_mode: false,
            editing_provider: None,
            test_result: None,
            loading: false,
            tick: 0,
            status_msg: String::new(),
            endpoint_form: EndpointForm::default(),
            endpoint_form_active: false,
            endpoint_form_field: 0,
            modality_form: ModalityForm::default(),
            modality_form_active: false,
            modality_form_field: 0,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SettingsAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return SettingsAction::Continue;
        }

        // Provider key input mode
        if self.input_mode {
            return self.handle_input(key);
        }

        // Endpoint form mode
        if self.endpoint_form_active {
            return self.handle_endpoint_form(key);
        }

        // Modality form mode
        if self.modality_form_active {
            return self.handle_modality_form(key);
        }

        // Sub-tab switching
        match key.code {
            KeyCode::Char('1') => {
                self.sub = SettingsSub::Providers;
                return SettingsAction::RefreshProviders;
            }
            KeyCode::Char('2') => {
                self.sub = SettingsSub::Endpoints;
                return SettingsAction::RefreshModels;
            }
            KeyCode::Char('3') => {
                self.sub = SettingsSub::Modalities;
                return SettingsAction::RefreshModels;
            }
            _ => {}
        }

        match self.sub {
            SettingsSub::Providers => self.handle_providers(key),
            SettingsSub::Endpoints => self.handle_endpoints(key),
            SettingsSub::Modalities => self.handle_modalities(key),
        }
    }

    fn handle_input(&mut self, key: KeyEvent) -> SettingsAction {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = false;
                self.editing_provider = None;
                self.input_buf.clear();
            }
            KeyCode::Enter => {
                self.input_mode = false;
                if let Some(name) = self.editing_provider.take() {
                    if !self.input_buf.is_empty() {
                        let api_key = self.input_buf.clone();
                        self.input_buf.clear();
                        return SettingsAction::SaveProviderKey { name, key: api_key };
                    }
                }
                self.input_buf.clear();
            }
            KeyCode::Backspace => { self.input_buf.pop(); }
            KeyCode::Char(c) => { self.input_buf.push(c); }
            _ => {}
        }
        SettingsAction::Continue
    }

    fn handle_endpoint_form(&mut self, key: KeyEvent) -> SettingsAction {
        let fields = &mut [
            &mut self.endpoint_form.name,
            &mut self.endpoint_form.provider,
            &mut self.endpoint_form.model,
            &mut self.endpoint_form.base_url,
            &mut self.endpoint_form.format,
        ];
        let max_field = fields.len() - 1;
        match key.code {
            KeyCode::Esc => {
                self.endpoint_form_active = false;
                self.endpoint_form = EndpointForm::default();
            }
            KeyCode::Tab => {
                self.endpoint_form_field = if self.endpoint_form_field >= max_field { 0 } else { self.endpoint_form_field + 1 };
            }
            KeyCode::BackTab => {
                self.endpoint_form_field = if self.endpoint_form_field == 0 { max_field } else { self.endpoint_form_field - 1 };
            }
            KeyCode::Enter => {
                let form = std::mem::take(&mut self.endpoint_form);
                self.endpoint_form_active = false;
                if !form.name.is_empty() && !form.provider.is_empty() && !form.model.is_empty() {
                    return SettingsAction::AddEndpoint {
                        name: form.name,
                        provider: form.provider,
                        model: form.model,
                        base_url: if form.base_url.is_empty() { "https://api.example.com/v1".to_string() } else { form.base_url },
                        format: if form.format.is_empty() { "openai".to_string() } else { form.format },
                    };
                }
            }
            KeyCode::Backspace => { fields[self.endpoint_form_field].pop(); }
            KeyCode::Char(c) => { fields[self.endpoint_form_field].push(c); }
            _ => {}
        }
        SettingsAction::Continue
    }

    fn handle_modality_form(&mut self, key: KeyEvent) -> SettingsAction {
        let fields = &mut [
            &mut self.modality_form.name,
            &mut self.modality_form.primary,
            &mut self.modality_form.fallbacks,
        ];
        let max_field = fields.len() - 1;
        match key.code {
            KeyCode::Esc => {
                self.modality_form_active = false;
                self.modality_form = ModalityForm::default();
            }
            KeyCode::Tab => {
                self.modality_form_field = if self.modality_form_field >= max_field { 0 } else { self.modality_form_field + 1 };
            }
            KeyCode::BackTab => {
                self.modality_form_field = if self.modality_form_field == 0 { max_field } else { self.modality_form_field - 1 };
            }
            KeyCode::Enter => {
                let form = std::mem::take(&mut self.modality_form);
                self.modality_form_active = false;
                if !form.name.is_empty() && !form.primary.is_empty() {
                    let fallbacks: Vec<String> = if form.fallbacks.is_empty() {
                        Vec::new()
                    } else {
                        form.fallbacks.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
                    };
                    return SettingsAction::AddModality {
                        name: form.name,
                        primary: form.primary,
                        fallbacks,
                    };
                }
            }
            KeyCode::Backspace => { fields[self.modality_form_field].pop(); }
            KeyCode::Char(c) => { fields[self.modality_form_field].push(c); }
            _ => {}
        }
        SettingsAction::Continue
    }

    fn handle_providers(&mut self, key: KeyEvent) -> SettingsAction {
        let total = self.providers.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if total > 0 {
                    let i = self.provider_list.selected().unwrap_or(0);
                    let next = if i == 0 { total - 1 } else { i - 1 };
                    self.provider_list.select(Some(next));
                    self.test_result = None;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if total > 0 {
                    let i = self.provider_list.selected().unwrap_or(0);
                    let next = (i + 1) % total;
                    self.provider_list.select(Some(next));
                    self.test_result = None;
                }
            }
            KeyCode::Char('e') => {
                if let Some(sel) = self.provider_list.selected() {
                    if sel < self.providers.len() {
                        self.editing_provider = Some(self.providers[sel].name.clone());
                        self.input_mode = true;
                        self.input_buf.clear();
                    }
                }
            }
            KeyCode::Char('d') => {
                if let Some(sel) = self.provider_list.selected() {
                    if sel < self.providers.len() {
                        return SettingsAction::DeleteProviderKey(self.providers[sel].name.clone());
                    }
                }
            }
            KeyCode::Char('t') => {
                if let Some(sel) = self.provider_list.selected() {
                    if sel < self.providers.len() {
                        self.test_result = None;
                        return SettingsAction::TestProvider(self.providers[sel].name.clone());
                    }
                }
            }
            KeyCode::Char('r') => return SettingsAction::RefreshProviders,
            _ => {}
        }
        SettingsAction::Continue
    }

    fn handle_endpoints(&mut self, key: KeyEvent) -> SettingsAction {
        let total = self.endpoints.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if total > 0 {
                    let i = self.endpoint_list.selected().unwrap_or(0);
                    let next = if i == 0 { total - 1 } else { i - 1 };
                    self.endpoint_list.select(Some(next));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if total > 0 {
                    let i = self.endpoint_list.selected().unwrap_or(0);
                    let next = (i + 1) % total;
                    self.endpoint_list.select(Some(next));
                }
            }
            KeyCode::Char('a') => {
                self.endpoint_form = EndpointForm::default();
                self.endpoint_form_active = true;
                self.endpoint_form_field = 0;
            }
            KeyCode::Char('d') => {
                if let Some(sel) = self.endpoint_list.selected() {
                    if sel < self.endpoints.len() {
                        return SettingsAction::DeleteEndpoint(self.endpoints[sel].name.clone());
                    }
                }
            }
            KeyCode::Char('r') => return SettingsAction::RefreshModels,
            _ => {}
        }
        SettingsAction::Continue
    }

    fn handle_modalities(&mut self, key: KeyEvent) -> SettingsAction {
        let total = self.modalities.len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if total > 0 {
                    let i = self.modality_list.selected().unwrap_or(0);
                    let next = if i == 0 { total - 1 } else { i - 1 };
                    self.modality_list.select(Some(next));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if total > 0 {
                    let i = self.modality_list.selected().unwrap_or(0);
                    let next = (i + 1) % total;
                    self.modality_list.select(Some(next));
                }
            }
            KeyCode::Char('a') => {
                self.modality_form = ModalityForm::default();
                self.modality_form_active = true;
                self.modality_form_field = 0;
            }
            KeyCode::Char('d') => {
                if let Some(sel) = self.modality_list.selected() {
                    if sel < self.modalities.len() {
                        return SettingsAction::DeleteModality(self.modalities[sel].name.clone());
                    }
                }
            }
            KeyCode::Char('s') => {
                if let Some(sel) = self.modality_list.selected() {
                    if sel < self.modalities.len() {
                        return SettingsAction::SetDefaultModality(self.modalities[sel].name.clone());
                    }
                }
            }
            KeyCode::Char('r') => return SettingsAction::RefreshModels,
            _ => {}
        }
        SettingsAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut SettingsState) {
    let block = Block::default()
        .title(Line::from(vec![Span::styled(" Settings ", theme::title_style())]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT))
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(inner);

    draw_sub_tabs(f, chunks[0], state.sub);

    let sep = "\u{2500}".repeat(chunks[1].width as usize);
    f.render_widget(Paragraph::new(Span::styled(sep, theme::dim_style())), chunks[1]);

    match state.sub {
        SettingsSub::Providers => draw_providers(f, chunks[2], state),
        SettingsSub::Endpoints => draw_endpoints(f, chunks[2], state),
        SettingsSub::Modalities => draw_modalities(f, chunks[2], state),
    }

    let hint_text = match state.sub {
        SettingsSub::Providers if state.input_mode => "  [Enter] Save  [Esc] Cancel",
        SettingsSub::Providers => "  [\u{2191}\u{2193}] Navigate  [e] Set Key  [d] Delete  [t] Test  [r] Refresh",
        SettingsSub::Endpoints if state.endpoint_form_active => "  [Tab] Next Field  [Enter] Submit  [Esc] Cancel",
        SettingsSub::Endpoints => "  [\u{2191}\u{2193}] Navigate  [a] Add  [d] Delete  [r] Refresh",
        SettingsSub::Modalities if state.modality_form_active => "  [Tab] Next Field  [Enter] Submit  [Esc] Cancel",
        SettingsSub::Modalities => "  [\u{2191}\u{2193}] Navigate  [a] Add  [d] Delete  [s] Set Default  [r] Refresh",
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(hint_text, theme::hint_style())])),
        chunks[3],
    );

    // Form overlays
    if state.endpoint_form_active {
        draw_endpoint_form(f, inner, state);
    }
    if state.modality_form_active {
        draw_modality_form(f, inner, state);
    }
}

fn draw_sub_tabs(f: &mut Frame, area: Rect, active: SettingsSub) {
    let tabs = [
        (SettingsSub::Providers, "1 Providers"),
        (SettingsSub::Endpoints, "2 Endpoints"),
        (SettingsSub::Modalities, "3 Modalities"),
    ];
    let mut spans = vec![Span::raw("  ")];
    for (sub, label) in &tabs {
        let style = if *sub == active { theme::tab_active() } else { theme::tab_inactive() };
        spans.push(Span::styled(format!(" {label} "), style));
        spans.push(Span::raw(" "));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ── Providers tab ───────────────────────────────────────────────────────────

fn draw_providers(f: &mut Frame, area: Rect, state: &mut SettingsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(2),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {:<20} {:<20} {}", "Provider", "Status", "Env Variable"),
            theme::table_header(),
        )])),
        chunks[0],
    );

    if state.loading && state.providers.is_empty() {
        let spinner = theme::SPINNER_FRAMES[state.tick % theme::SPINNER_FRAMES.len()];
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("  {spinner} "), Style::default().fg(theme::CYAN)),
                Span::styled("Loading providers\u{2026}", theme::dim_style()),
            ])),
            chunks[1],
        );
    } else if state.providers.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("  No providers available.", theme::dim_style())),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state.providers.iter().map(|p| {
            let (badge, badge_style) = if p.is_local {
                match p.reachable {
                    Some(true) => {
                        let ms = p.latency_ms.unwrap_or(0);
                        (format!("\u{2714} Online ({ms}ms)"), Style::default().fg(theme::GREEN))
                    }
                    Some(false) => ("\u{2718} Offline".to_string(), Style::default().fg(theme::RED)),
                    None => ("\u{25cb} Local".to_string(), theme::dim_style()),
                }
            } else if p.configured {
                ("\u{2714} Configured".to_string(), Style::default().fg(theme::GREEN))
            } else {
                ("\u{25cb} Not set".to_string(), theme::dim_style())
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("  {:<20}", &p.name), Style::default().fg(theme::CYAN)),
                Span::styled(format!(" {::<20}", badge), badge_style),
                Span::styled(format!(" {}", &p.env_var), theme::dim_style()),
            ]))
        }).collect();

        let list = List::new(items)
            .highlight_style(theme::selected_style())
            .highlight_symbol("> ");
        f.render_stateful_widget(list, chunks[1], &mut state.provider_list);
    }

    // Input / test result / status
    if state.input_mode {
        let provider_name = state.editing_provider.as_deref().unwrap_or("?");
        f.render_widget(
            Paragraph::new(vec![
                Line::from(vec![Span::styled(
                    format!("  Enter API key for {provider_name}: "),
                    Style::default().fg(theme::YELLOW),
                )]),
                Line::from(vec![
                    Span::raw("  > "),
                    Span::styled("\u{2022}".repeat(state.input_buf.len().min(40)), theme::input_style()),
                    Span::styled("\u{2588}", Style::default().fg(theme::GREEN).add_modifier(Modifier::SLOW_BLINK)),
                ]),
            ]),
            chunks[2],
        );
    } else if let Some(result) = &state.test_result {
        let (icon, style) = if result.success {
            ("\u{2714}", Style::default().fg(theme::GREEN))
        } else {
            ("\u{2718}", Style::default().fg(theme::RED))
        };
        f.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(format!("  {icon} "), style),
                    Span::styled(format!("{}: {}", result.provider, result.message), style),
                ]),
                Line::from(vec![Span::styled(
                    if result.success { format!("  Latency: {}ms", result.latency_ms) } else { String::new() },
                    theme::dim_style(),
                )]),
            ]),
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
    }
}

// ── Endpoints tab ───────────────────────────────────────────────────────────

fn draw_endpoints(f: &mut Frame, area: Rect, state: &mut SettingsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {:<20} {:<10} {:<18} {:<10} {}", "Endpoint", "Provider", "Model", "Format", "Status"),
            theme::table_header(),
        )])),
        chunks[0],
    );

    if state.loading && state.endpoints.is_empty() {
        let spinner = theme::SPINNER_FRAMES[state.tick % theme::SPINNER_FRAMES.len()];
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("  {spinner} "), Style::default().fg(theme::CYAN)),
                Span::styled("Loading endpoints\u{2026}", theme::dim_style()),
            ])),
            chunks[1],
        );
    } else if state.endpoints.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("  No endpoints. Press [a] to add.", theme::dim_style())),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state.endpoints.iter().map(|e| {
            let (status, status_style) = if e.ready {
                ("\u{2714} Ready", Style::default().fg(theme::GREEN))
            } else {
                ("\u{2718} Down", Style::default().fg(theme::RED))
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("  {:<20}", truncate(&e.name, 19)), Style::default().fg(theme::CYAN)),
                Span::styled(format!(" {::<10}", truncate(&e.provider, 9)), theme::dim_style()),
                Span::styled(format!(" {:<18}", truncate(&e.model, 17)), Style::default().fg(theme::YELLOW)),
                Span::styled(format!(" {:<10}", &e.format), theme::dim_style()),
                Span::styled(format!(" {status}"), status_style),
            ]))
        }).collect();

        let list = List::new(items)
            .highlight_style(theme::selected_style())
            .highlight_symbol("> ");
        f.render_stateful_widget(list, chunks[1], &mut state.endpoint_list);
    }

    if !state.status_msg.is_empty() && !state.endpoint_form_active {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!("  {}", state.status_msg),
                Style::default().fg(theme::GREEN),
            )])),
            chunks[2],
        );
    }
}

// ── Modalities tab ──────────────────────────────────────────────────────────

fn draw_modalities(f: &mut Frame, area: Rect, state: &mut SettingsState) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("  {:<14} {:<20} {}  {}", "Modality", "Primary", "Fallbacks", "Default"),
            theme::table_header(),
        )])),
        chunks[0],
    );

    if state.loading && state.modalities.is_empty() {
        let spinner = theme::SPINNER_FRAMES[state.tick % theme::SPINNER_FRAMES.len()];
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("  {spinner} "), Style::default().fg(theme::CYAN)),
                Span::styled("Loading modalities\u{2026}", theme::dim_style()),
            ])),
            chunks[1],
        );
    } else if state.modalities.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("  No modalities. Press [a] to add.", theme::dim_style())),
            chunks[1],
        );
    } else {
        let default_mod = ""; // TODO: pass default_modality into state if needed
        let items: Vec<ListItem> = state.modalities.iter().map(|m| {
            let fallback_str = if m.fallbacks.is_empty() { String::new() } else { m.fallbacks.join(", ") };
            let default_badge = if m.name == default_mod { " *" } else { "" };
            ListItem::new(Line::from(vec![
                Span::styled(format!("  {:<14}", &m.name), Style::default().fg(theme::ACCENT)),
                Span::styled(format!(" {:<20}", &m.primary), Style::default().fg(theme::GREEN)),
                Span::styled(format!(" {fallback_str}"), theme::dim_style()),
                Span::styled(default_badge, Style::default().fg(theme::YELLOW).add_modifier(Modifier::BOLD)),
            ]))
        }).collect();

        let list = List::new(items)
            .highlight_style(theme::selected_style())
            .highlight_symbol("> ");
        f.render_stateful_widget(list, chunks[1], &mut state.modality_list);
    }

    if !state.status_msg.is_empty() && !state.modality_form_active {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!("  {}", state.status_msg),
                Style::default().fg(theme::GREEN),
            )])),
            chunks[2],
        );
    }
}

// ── Endpoint form overlay ───────────────────────────────────────────────────

fn draw_endpoint_form(f: &mut Frame, area: Rect, state: &SettingsState) {
    let w = area.width.clamp(40, 60).min(area.width);
    let h = 9;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);

    f.render_widget(Clear, popup);
    let block = Block::default()
        .title(Line::from(vec![Span::styled(" Add Endpoint ", theme::title_style())]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT))
        .padding(Padding::horizontal(1));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let fields = [
        ("Name", &state.endpoint_form.name),
        ("Provider", &state.endpoint_form.provider),
        ("Model", &state.endpoint_form.model),
        ("Base URL", &state.endpoint_form.base_url),
        ("Format", &state.endpoint_form.format),
    ];

    let mut lines: Vec<Line> = Vec::new();
    for (i, (label, value)) in fields.iter().enumerate() {
        let active = i == state.endpoint_form_field;
        let label_style = if active { Style::default().fg(theme::YELLOW) } else { theme::dim_style() };
        let cursor = if active { "\u{2588}" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(format!(" {label}: "), label_style),
            Span::styled((*value).clone(), Style::default().fg(theme::TEXT_PRIMARY)),
            Span::styled(cursor, Style::default().fg(theme::ACCENT)),
        ]));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

// ── Modality form overlay ───────────────────────────────────────────────────

fn draw_modality_form(f: &mut Frame, area: Rect, state: &SettingsState) {
    let w = area.width.clamp(40, 50).min(area.width);
    let h = 7;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);

    f.render_widget(Clear, popup);
    let block = Block::default()
        .title(Line::from(vec![Span::styled(" Add Modality ", theme::title_style())]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT))
        .padding(Padding::horizontal(1));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let fields = [
        ("Name", &state.modality_form.name),
        ("Primary Endpoint", &state.modality_form.primary),
        ("Fallbacks (comma)", &state.modality_form.fallbacks),
    ];

    let mut lines: Vec<Line> = Vec::new();
    for (i, (label, value)) in fields.iter().enumerate() {
        let active = i == state.modality_form_field;
        let label_style = if active { Style::default().fg(theme::YELLOW) } else { theme::dim_style() };
        let cursor = if active { "\u{2588}" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(format!(" {label}: "), label_style),
            Span::styled((*value).clone(), Style::default().fg(theme::TEXT_PRIMARY)),
            Span::styled(cursor, Style::default().fg(theme::ACCENT)),
        ]));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}\u{2026}", opencarrier_types::truncate_str(s, max.saturating_sub(1)))
    }
}
