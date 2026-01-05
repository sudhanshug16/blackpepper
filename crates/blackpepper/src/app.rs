use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Terminal;

use crate::commands::{command_hint_lines, complete_command_input, parse_command, run_command, CommandContext};
use crate::config::{load_config, Config};
use crate::events::AppEvent;
use crate::git::resolve_repo_root;
use crate::keymap::{matches_chord, parse_key_chord, KeyChord};
use crate::state::{get_active_workspace, load_state, record_active_workspace};
use crate::terminal::{key_event_to_bytes, mouse_event_to_bytes, TerminalSession};
use crate::workspaces::{list_workspace_names, workspace_absolute_path, workspace_name_from_path};

const OUTPUT_MAX_LINES: usize = 6;
const BOTTOM_HORIZONTAL_PADDING: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Work,
    Manage,
}

#[derive(Debug, Default)]
struct WorkspaceOverlay {
    visible: bool,
    items: Vec<String>,
    message: Option<String>,
    selected: usize,
}

#[derive(Debug, Default)]
struct TabOverlay {
    visible: bool,
    items: Vec<String>,
    selected: usize,
}

struct WorkspaceTab {
    id: u64,
    name: String,
    terminal: TerminalSession,
}

struct WorkspaceTabs {
    tabs: Vec<WorkspaceTab>,
    active: usize,
    next_index: usize,
}

impl WorkspaceTabs {
    fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            next_index: 1,
        }
    }
}

struct App {
    mode: Mode,
    command_active: bool,
    command_input: String,
    output: Option<String>,
    cwd: PathBuf,
    repo_root: Option<PathBuf>,
    active_workspace: Option<String>,
    toggle_chord: Option<KeyChord>,
    switch_chord: Option<KeyChord>,
    switch_tab_chord: Option<KeyChord>,
    should_quit: bool,
    config: Config,
    tabs: HashMap<String, WorkspaceTabs>,
    overlay: WorkspaceOverlay,
    tab_overlay: TabOverlay,
    event_tx: Sender<AppEvent>,
    terminal_seq: u64,
    tab_bar_area: Option<Rect>,
    mouse_debug: bool,
    loading: Option<String>,
}

pub fn run() -> io::Result<()> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(EnableMouseCapture)?;
    stdout.write_all(b"\x1b[?1000h\x1b[?1006h")?;
    stdout.flush()?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal);

    disable_raw_mode()?;
    terminal.backend_mut().execute(DisableMouseCapture)?;
    terminal.backend_mut().write_all(b"\x1b[?1000l\x1b[?1006l")?;
    terminal.backend_mut().flush()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let (event_tx, event_rx) = mpsc::channel::<AppEvent>();
    spawn_input_thread(event_tx.clone());

    let mut app = App::new(event_tx.clone());
    terminal.draw(|frame| app.render(frame))?;

    while !app.should_quit {
        let event = match event_rx.recv() {
            Ok(event) => event,
            Err(_) => break,
        };
        app.handle_event(event);
        while let Ok(event) = event_rx.try_recv() {
            app.handle_event(event);
        }

        terminal.draw(|frame| app.render(frame))?;
    }
    Ok(())
}

impl App {
    fn new(event_tx: Sender<AppEvent>) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let config = load_config(&cwd);
        let toggle_chord = parse_key_chord(&config.keymap.toggle_mode);
        let switch_chord = parse_key_chord(&config.keymap.switch_workspace);
        let switch_tab_chord = parse_key_chord(&config.keymap.switch_tab);

        let mut repo_root = resolve_repo_root(&cwd);
        let mut active_workspace = None;
        let mut app_cwd = cwd.clone();

        if let (Some(root), Some(state)) = (repo_root.as_ref(), load_state()) {
            if let Some(path) = get_active_workspace(&state, root) {
                let path_string = path.to_string_lossy().to_string();
                if let Some(name) = workspace_name_from_path(&path_string) {
                    active_workspace = Some(name);
                    app_cwd = path;
                    repo_root = resolve_repo_root(&app_cwd).or(repo_root);
                }
            }
        }

        let mut app = Self {
            mode: Mode::Work,
            command_active: false,
            command_input: String::new(),
            output: None,
            cwd: app_cwd,
            repo_root,
            active_workspace,
            toggle_chord,
            switch_chord,
            switch_tab_chord,
            should_quit: false,
            config,
            tabs: HashMap::new(),
            overlay: WorkspaceOverlay::default(),
            tab_overlay: TabOverlay::default(),
            event_tx,
            terminal_seq: 0,
            tab_bar_area: None,
            mouse_debug: false,
            loading: None,
        };

        if let Err(err) = app.ensure_active_workspace_tabs(24, 80) {
            app.set_output(err);
        }

        app
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Input(key) => self.handle_key(key),
            AppEvent::PtyOutput(id, bytes) => {
                self.process_terminal_output(id, &bytes);
            }
            AppEvent::Mouse(mouse) => {
                if self.loading.is_some() {
                    return;
                }
                if self.handle_mouse(mouse) {
                    return;
                }
                if self.mode == Mode::Work && !self.command_active && !self.overlay_visible() {
                    self.send_mouse_to_active_terminal(mouse);
                }
            }
            AppEvent::Resize(_, _) => {}
            AppEvent::CommandDone { name, args, result } => {
                self.loading = None;
                self.set_output(result.message);
                if name == "workspace" {
                    if let Some(subcommand) = args.get(0) {
                        if subcommand == "create" && result.ok {
                            if let Some(name) = result.data.as_deref() {
                                if self.set_active_workspace(name).is_ok() {
                                    self.set_output(format!("Active workspace: {name}"));
                                }
                            }
                            self.mode = Mode::Work;
                        }
                        if subcommand == "destroy" && result.ok {
                            if let Some(name) = args.get(1) {
                                if self.active_workspace.as_deref() == Some(name.as_str()) {
                                    self.active_workspace = None;
                                    self.tabs.remove(name);
                                    if let Some(root) = &self.repo_root {
                                        self.cwd = root.clone();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn render(&mut self, frame: &mut ratatui::Frame) {
        let area = frame.area();
        let output_lines = self.output_lines_owned(area.width as usize);
        let output_height = output_lines.len() as u16;
        let separator_height = 1u16;
        let command_height = 1u16;

        let split_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(separator_height),
                Constraint::Length(output_height),
                Constraint::Length(command_height),
            ])
            .split(area);

        self.render_work_area(frame, split_chunks[0]);
        self.render_separator(frame, split_chunks[1], area.width as usize);

        if output_height > 0 {
            let output_area = inset_horizontal(split_chunks[2], BOTTOM_HORIZONTAL_PADDING);
            let output = Paragraph::new(output_lines).style(Style::default().fg(Color::Gray));
            frame.render_widget(output, output_area);
        }

        let command_area = inset_horizontal(split_chunks[3], BOTTOM_HORIZONTAL_PADDING);
        self.render_command_bar(frame, command_area);

        if self.overlay.visible {
            self.render_overlay(frame, area);
        }
        if self.tab_overlay.visible {
            self.render_tab_overlay(frame, area);
        }
        if let Some(message) = &self.loading {
            self.render_loader(frame, area, message);
        }
    }

    fn render_work_area(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let Some(workspace) = self.active_workspace.as_deref() else {
            self.tab_bar_area = None;
            let body_lines = self.body_lines();
            frame.render_widget(Paragraph::new(body_lines), area);
            return;
        };

        let Some(tabs) = self.tabs.get(workspace) else {
            self.tab_bar_area = None;
            let body_lines = self.body_lines();
            frame.render_widget(Paragraph::new(body_lines), area);
            return;
        };

        if tabs.tabs.is_empty() {
            self.tab_bar_area = None;
            let body_lines = self.body_lines();
            frame.render_widget(Paragraph::new(body_lines), area);
            return;
        }

        let tab_height = 1u16;
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(tab_height), Constraint::Min(1)])
            .split(area);

        self.render_tab_bar(frame, chunks[0], tabs);
        self.tab_bar_area = Some(chunks[0]);

        if let Some(tab) = self.active_tab_mut() {
            let terminal = &mut tab.terminal;
            terminal.resize(chunks[1].height, chunks[1].width);
            let lines = terminal.render_lines(chunks[1].height, chunks[1].width);
            frame.render_widget(Paragraph::new(lines), chunks[1]);
        }
    }

    fn render_overlay(&self, frame: &mut ratatui::Frame, area: Rect) {
        let overlay_rect = centered_rect(60, 50, area);
        let mut lines = Vec::new();
        if let Some(message) = &self.overlay.message {
            lines.push(Line::raw(message.clone()));
        } else {
            for (idx, name) in self.overlay.items.iter().enumerate() {
                let mut label = name.clone();
                if self.active_workspace.as_deref() == Some(name.as_str()) {
                    label = format!("{label} (active)");
                }
                let style = if idx == self.overlay.selected {
                    Style::default().fg(Color::Black).bg(Color::White)
                } else {
                    Style::default().fg(Color::White)
                };
                lines.push(Line::from(Span::styled(label, style)));
            }
        }

        let block = Block::default().borders(Borders::ALL).title("Workspaces");
        frame.render_widget(Paragraph::new(lines).block(block), overlay_rect);
    }

    fn render_tab_overlay(&self, frame: &mut ratatui::Frame, area: Rect) {
        let overlay_rect = centered_rect(50, 40, area);
        let mut lines = Vec::new();
        let active_index = self
            .active_workspace
            .as_deref()
            .and_then(|workspace| self.tabs.get(workspace))
            .map(|tabs| tabs.active);
        for (idx, name) in self.tab_overlay.items.iter().enumerate() {
            let mut label = name.clone();
            if Some(idx) == active_index {
                label = format!("{label} (active)");
            }
            let style = if idx == self.tab_overlay.selected {
                Style::default().fg(Color::Black).bg(Color::White)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(Span::styled(label, style)));
        }
        let block = Block::default().borders(Borders::ALL).title("Tabs");
        frame.render_widget(Paragraph::new(lines).block(block), overlay_rect);
    }

    fn render_loader(&self, frame: &mut ratatui::Frame, area: Rect, message: &str) {
        let rect = centered_rect(60, 20, area);
        let lines = vec![
            Line::from(Span::styled("Working...", Style::default().fg(Color::White))),
            Line::raw(""),
            Line::from(Span::styled(message, Style::default().fg(Color::DarkGray))),
        ];
        frame.render_widget(Paragraph::new(lines), rect);
    }

    fn body_lines(&self) -> Vec<Line<'_>> {
        let mut lines = Vec::new();
        lines.push(Line::from(vec![
            Span::styled("Blackpepper", Style::default().fg(Color::Yellow)),
            Span::raw("  Rust TUI"),
        ]));
        lines.push(Line::raw(""));
        if let Some(workspace) = &self.active_workspace {
            lines.push(Line::raw(format!("Active workspace: {workspace}")));
            lines.push(Line::raw(format!("Path: {}", self.cwd.to_string_lossy())));
        } else {
            lines.push(Line::raw("No workspace selected."));
            lines.push(Line::raw("Use Ctrl+P or :workspace."));
        }
        lines
    }

    fn output_lines_owned(&self, _width: usize) -> Vec<Line<'static>> {
        if self.command_active {
            let hints = command_hint_lines(&self.command_input, OUTPUT_MAX_LINES);
            let mut lines = Vec::new();
            if hints.is_empty() {
                lines.push(Line::raw("No commands found."));
            } else {
                for line in hints {
                    lines.push(Line::raw(line));
                    if lines.len() >= OUTPUT_MAX_LINES {
                        break;
                    }
                }
            }
            return lines;
        }

        let Some(message) = self.output.as_ref() else {
            return Vec::new();
        };
        if message.trim().is_empty() {
            return Vec::new();
        }

        let mut lines = Vec::new();
        for line in message.lines() {
            lines.push(Line::raw(line.to_string()));
            if lines.len() >= OUTPUT_MAX_LINES {
                break;
            }
        }
        lines
    }

    fn dashed_line(&self, width: usize) -> String {
        if width == 0 {
            return String::new();
        }
        let pattern = "- ";
        pattern.repeat(width / pattern.len() + 1)[..width].to_string()
    }

    fn render_separator(&self, frame: &mut ratatui::Frame, area: Rect, width: usize) {
        let separator = Paragraph::new(Line::raw(self.dashed_line(width)))
            .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM));
        frame.render_widget(separator, area);
    }

    fn render_tab_bar(&self, frame: &mut ratatui::Frame, area: Rect, tabs: &WorkspaceTabs) {
        let mut spans = Vec::new();
        for (index, tab) in tabs.tabs.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw("  "));
            }
            let label = format!("{}:{}", index + 1, tab.name);
            let style = if index == tabs.active {
                Style::default().fg(Color::Black).bg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM)
            };
            spans.push(Span::styled(label, style));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn render_command_bar(&self, frame: &mut ratatui::Frame, area: Rect) {
        let workspace_name = self.active_workspace.as_deref().map(str::trim).filter(|name| !name.is_empty());
        if let Some(name) = workspace_name {
            let width = area.width as usize;
            let label_len = name.chars().count();
            if width > label_len + 1 {
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Min(1), Constraint::Length((label_len + 1) as u16)])
                    .split(area);

                let command_line = self.command_line();
                frame.render_widget(Paragraph::new(command_line), chunks[0]);

                let label = Paragraph::new(Line::from(Span::styled(
                    name.to_string(),
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
                )))
                .alignment(Alignment::Right);
                frame.render_widget(label, chunks[1]);
                return;
            }
        }

        let command_line = self.command_line();
        frame.render_widget(Paragraph::new(command_line), area);
    }

    fn command_line(&self) -> Line<'_> {
        if self.command_active {
            Line::from(vec![
                Span::styled(self.command_input.clone(), Style::default().fg(Color::White)),
                Span::styled(" ", Style::default().bg(Color::White).fg(Color::Black)),
            ])
        } else {
            let label = match self.mode {
                Mode::Work => "-- WORK --",
                Mode::Manage => "-- MANAGE --",
            };
            let style = if self.mode == Mode::Manage {
                Style::default().bg(Color::Magenta).fg(Color::Black)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Line::from(vec![Span::styled(label, style)])
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if self.loading.is_some() {
            return;
        }
        if self.overlay.visible {
            self.handle_overlay_key(key);
            return;
        }
        if self.tab_overlay.visible {
            self.handle_tab_overlay_key(key);
            return;
        }

        if self.command_active {
            self.handle_command_input(key);
            return;
        }

        if self.handle_tab_shortcut(key) {
            return;
        }

        if let Some(chord) = &self.toggle_chord {
            if matches_chord(key, chord) {
                self.mode = if self.mode == Mode::Work { Mode::Manage } else { Mode::Work };
                return;
            }
        }

        if let Some(chord) = &self.switch_chord {
            if matches_chord(key, chord) {
                self.open_workspace_overlay();
                return;
            }
        }

        if let Some(chord) = &self.switch_tab_chord {
            if matches_chord(key, chord) {
                self.open_tab_overlay();
                return;
            }
        }

        if self.mode == Mode::Manage && matches_command_open(key) {
            self.open_command();
            return;
        }

        if self.mode == Mode::Manage && key.code == KeyCode::Char('q') && key.modifiers.is_empty() {
            self.should_quit = true;
            return;
        }

        if self.mode == Mode::Manage && key.code == KeyCode::Esc {
            self.mode = Mode::Work;
            return;
        }

        if self.mode == Mode::Work {
            if let Some(terminal) = self.active_terminal_mut() {
                if let Some(bytes) = key_event_to_bytes(key) {
                    terminal.write_bytes(&bytes);
                }
            }
        }
    }

    fn handle_tab_shortcut(&mut self, key: KeyEvent) -> bool {
        if self.mode != Mode::Work {
            return false;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Tab {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                self.tab_prev();
            } else {
                self.tab_next();
            }
            return true;
        }

        if key.modifiers.contains(KeyModifiers::ALT) {
            if let KeyCode::Char(ch) = key.code {
                if ('1'..='9').contains(&ch) {
                    let index = ch.to_digit(10).unwrap_or(1) as usize;
                    self.tab_select(index.saturating_sub(1));
                    return true;
                }
            }
        }

        false
    }

    fn handle_workspace_command(&mut self, args: &[String]) {
        let Some(subcommand) = args.first() else {
            self.set_output("Usage: :workspace <list|switch|create|destroy>".to_string());
            return;
        };
        match subcommand.as_str() {
            "list" => {
                self.open_workspace_overlay();
            }
            "switch" => {
                let Some(name) = args.get(1) else {
                    self.set_output("Usage: :workspace switch <name>".to_string());
                    return;
                };
                match self.set_active_workspace(name) {
                    Ok(()) => self.set_output(format!("Active workspace: {name}")),
                    Err(err) => self.set_output(err),
                }
            }
            "create" | "destroy" => {
                if subcommand == "destroy" && args.len() == 1 {
                    if let Some(active) = self.active_workspace.as_ref() {
                        let mut args = args.to_vec();
                        args.push(active.clone());
                        self.start_command("workspace", args);
                        return;
                    }
                }
                self.start_command("workspace", args.to_vec());
            }
            _ => {
                self.set_output("Usage: :workspace <list|switch|create|destroy>".to_string());
            }
        }
    }

    fn handle_tab_command(&mut self, args: &[String]) {
        let Some(subcommand) = args.first() else {
            self.set_output("Usage: :tab <new|rename|close|next|prev|switch>".to_string());
            return;
        };

        match subcommand.as_str() {
            "new" => {
                let name = args.get(1).cloned();
                match self.create_tab_for_active(24, 80, name) {
                    Ok(name) => {
                        self.set_output(format!("Opened tab: {name}"));
                        self.mode = Mode::Work;
                    }
                    Err(err) => self.set_output(err),
                }
            }
            "rename" => {
                let Some(name) = args.get(1) else {
                    self.set_output("Usage: :tab rename <name>".to_string());
                    return;
                };
                match self.rename_active_tab(name) {
                    Ok(()) => self.set_output(format!("Renamed tab to {name}")),
                    Err(err) => self.set_output(err),
                }
            }
            "close" => match self.close_active_tab() {
                Ok(message) => self.set_output(message),
                Err(err) => self.set_output(err),
            },
            "next" => self.tab_next(),
            "prev" => self.tab_prev(),
            "switch" => {
                if let Some(arg) = args.get(1) {
                    self.tab_select_by_arg(arg);
                } else {
                    self.open_tab_overlay();
                }
            }
            _ => {
                if args.len() == 1 {
                    self.tab_select_by_arg(subcommand);
                } else {
                    self.set_output("Usage: :tab <new|rename|close|next|prev|switch>".to_string());
                }
            }
        }
    }

    fn handle_tab_overlay_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.close_tab_overlay();
            }
            KeyCode::Enter => {
                if let Some(name) = self.tab_overlay.items.get(self.tab_overlay.selected) {
                    let name = name.clone();
                    self.tab_select_by_arg(&name);
                }
                self.close_tab_overlay();
            }
            KeyCode::Up => {
                self.move_tab_overlay_selection(-1);
            }
            KeyCode::Down => {
                self.move_tab_overlay_selection(1);
            }
            KeyCode::Char('k') => {
                self.move_tab_overlay_selection(-1);
            }
            KeyCode::Char('j') => {
                self.move_tab_overlay_selection(1);
            }
            _ => {}
        }
    }

    fn move_tab_overlay_selection(&mut self, delta: isize) {
        if self.tab_overlay.items.is_empty() {
            return;
        }
        let len = self.tab_overlay.items.len() as isize;
        let mut next = self.tab_overlay.selected as isize + delta;
        if next < 0 {
            next = len - 1;
        } else if next >= len {
            next = 0;
        }
        self.tab_overlay.selected = next as usize;
    }

    fn open_tab_overlay(&mut self) {
        let Some(workspace) = self.active_workspace.as_deref() else {
            self.set_output("No active workspace.".to_string());
            return;
        };
        let Some(tabs) = self.tabs.get(workspace) else {
            self.set_output("No tabs for active workspace.".to_string());
            return;
        };
        if tabs.tabs.is_empty() {
            self.set_output("No tabs yet.".to_string());
            return;
        }
        self.tab_overlay.items = tabs.tabs.iter().map(|tab| tab.name.clone()).collect();
        self.tab_overlay.selected = tabs.active;
        self.tab_overlay.visible = true;
    }

    fn close_tab_overlay(&mut self) {
        self.tab_overlay.visible = false;
    }

    fn overlay_visible(&self) -> bool {
        self.overlay.visible || self.tab_overlay.visible
    }

    fn handle_debug_command(&mut self, args: &[String]) {
        let Some(subcommand) = args.first() else {
            self.set_output("Usage: :debug <mouse>".to_string());
            return;
        };
        match subcommand.as_str() {
            "mouse" => {
                self.mouse_debug = !self.mouse_debug;
                let state = if self.mouse_debug { "on" } else { "off" };
                self.set_output(format!("Mouse debug {state}."));
            }
            _ => {
                self.set_output("Usage: :debug <mouse>".to_string());
            }
        }
    }

    fn start_command(&mut self, name: &str, args: Vec<String>) {
        if self.loading.is_some() {
            self.set_output("Command already running.".to_string());
            return;
        }
        let label = if args.is_empty() {
            format!(":{name}")
        } else {
            format!(":{name} {}", args.join(" "))
        };
        self.loading = Some(label);
        let ctx = CommandContext {
            cwd: self.cwd.clone(),
        };
        let tx = self.event_tx.clone();
        let name = name.to_string();
        std::thread::spawn(move || {
            let result = run_command(&name, &args, &ctx);
            let _ = tx.send(AppEvent::CommandDone { name, args, result });
        });
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        if self.loading.is_some() {
            return false;
        }
        if self.command_active || self.overlay_visible() {
            return false;
        }

        if !matches!(
            mouse.kind,
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
        ) {
            return false;
        }

        let Some(workspace) = self.active_workspace.as_deref() else {
            return false;
        };
        let Some(tabs) = self.tabs.get_mut(workspace) else {
            return false;
        };
        if tabs.tabs.is_empty() {
            return false;
        }

        let mut cursor = 0u16;
        for (index, tab) in tabs.tabs.iter().enumerate() {
            let label = format!("{}:{}", index + 1, tab.name);
            let width = label.chars().count() as u16;
            if mouse.column >= cursor && mouse.column < cursor.saturating_add(width) {
                tabs.active = index;
                if self.mouse_debug {
                    self.set_output(format!("Mouse click: col={} -> tab {}", mouse.column, index + 1));
                }
                return true;
            }
            cursor = cursor.saturating_add(width);
            if index + 1 < tabs.tabs.len() {
                cursor = cursor.saturating_add(2);
            }
        }

        false
    }

    fn handle_overlay_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.close_workspace_overlay();
            }
            KeyCode::Enter => {
                if let Some(name) = self.overlay.items.get(self.overlay.selected) {
                    let name = name.clone();
                    match self.set_active_workspace(&name) {
                        Ok(()) => self.set_output(format!("Active workspace: {name}")),
                        Err(err) => self.set_output(err),
                    }
                }
                self.close_workspace_overlay();
            }
            KeyCode::Up => {
                self.move_overlay_selection(-1);
            }
            KeyCode::Down => {
                self.move_overlay_selection(1);
            }
            KeyCode::Char('k') => {
                self.move_overlay_selection(-1);
            }
            KeyCode::Char('j') => {
                self.move_overlay_selection(1);
            }
            _ => {}
        }
    }

    fn move_overlay_selection(&mut self, delta: isize) {
        if self.overlay.items.is_empty() {
            return;
        }
        let len = self.overlay.items.len() as isize;
        let mut next = self.overlay.selected as isize + delta;
        if next < 0 {
            next = len - 1;
        } else if next >= len {
            next = 0;
        }
        self.overlay.selected = next as usize;
    }

    fn open_workspace_overlay(&mut self) {
        let root = match &self.repo_root {
            Some(root) => root.clone(),
            None => {
                self.overlay.message = Some("Not inside a git repository.".to_string());
                self.overlay.items.clear();
                self.overlay.selected = 0;
                self.overlay.visible = true;
                return;
            }
        };

        let names = list_workspace_names(&root);
        if names.is_empty() {
            self.overlay.message = Some("No workspaces yet.".to_string());
            self.overlay.items.clear();
            self.overlay.selected = 0;
        } else {
            self.overlay.message = None;
            self.overlay.items = names;
            if let Some(active) = &self.active_workspace {
                if let Some(index) = self.overlay.items.iter().position(|name| name == active) {
                    self.overlay.selected = index;
                } else {
                    self.overlay.selected = 0;
                }
            } else {
                self.overlay.selected = 0;
            }
        }

        self.overlay.visible = true;
    }

    fn close_workspace_overlay(&mut self) {
        self.overlay.visible = false;
    }

    fn handle_command_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.command_active = false;
                self.command_input.clear();
            }
            KeyCode::Enter => {
                let input = self.command_input.clone();
                self.command_active = false;
                self.command_input.clear();
                self.execute_command(&input);
            }
            KeyCode::Tab => {
                if let Some(completed) = complete_command_input(&self.command_input) {
                    self.command_input = completed;
                }
            }
            KeyCode::Backspace => {
                self.command_input.pop();
                if self.command_input.is_empty() {
                    self.command_active = false;
                }
            }
            KeyCode::Char(ch) => {
                self.command_input.push(ch);
            }
            _ => {}
        }
    }

    fn open_command(&mut self) {
        self.command_active = true;
        self.command_input = ":".to_string();
    }

    fn execute_command(&mut self, raw: &str) {
        let parsed = match parse_command(raw) {
            Ok(parsed) => parsed,
            Err(err) => {
                self.set_output(format!("Error: {}", err.error));
                return;
            }
        };

        if parsed.name == "quit" || parsed.name == "q" {
            self.should_quit = true;
            return;
        }

        if parsed.name == "workspace" {
            self.handle_workspace_command(&parsed.args);
            return;
        }

        if parsed.name == "tab" {
            self.handle_tab_command(&parsed.args);
            return;
        }

        if parsed.name == "pr" {
            self.start_command(&parsed.name, parsed.args.clone());
            return;
        }

        if parsed.name == "debug" {
            self.handle_debug_command(&parsed.args);
            return;
        }

        let result = run_command(
            &parsed.name,
            &parsed.args,
            &CommandContext {
                cwd: self.cwd.clone(),
            },
        );
        self.set_output(result.message);
    }

    fn set_active_workspace(&mut self, name: &str) -> Result<(), String> {
        if self.active_workspace.as_deref() == Some(name) {
            return Ok(());
        }
        let root = match &self.repo_root {
            Some(root) => root.clone(),
            None => return Err("Not inside a git repository.".to_string()),
        };
        let names = list_workspace_names(&root);
        if !names.iter().any(|entry| entry == name) {
            return Err(format!("Workspace '{name}' not found."));
        }
        let workspace_path = workspace_absolute_path(&root, name);
        self.cwd = workspace_path.clone();
        self.active_workspace = Some(name.to_string());
        let _ = record_active_workspace(&root, &workspace_path);
        self.config = load_config(&self.cwd);
        self.ensure_active_workspace_tabs(24, 80)
    }

    fn set_output(&mut self, message: String) {
        let trimmed = message.trim().to_string();
        if trimmed.is_empty() {
            self.output = None;
        } else {
            self.output = Some(trimmed);
        }
    }

    fn active_terminal_mut(&mut self) -> Option<&mut TerminalSession> {
        let workspace = self.active_workspace.as_deref()?;
        let tabs = self.tabs.get_mut(workspace)?;
        let index = tabs.active;
        tabs.tabs.get_mut(index).map(|tab| &mut tab.terminal)
    }

    fn active_tab_mut(&mut self) -> Option<&mut WorkspaceTab> {
        let workspace = self.active_workspace.as_deref()?;
        let tabs = self.tabs.get_mut(workspace)?;
        let index = tabs.active;
        tabs.tabs.get_mut(index)
    }

    fn process_terminal_output(&mut self, id: u64, bytes: &[u8]) {
        for tabs in self.tabs.values_mut() {
            for tab in &mut tabs.tabs {
                if tab.id == id {
                    tab.terminal.process_bytes(bytes);
                    return;
                }
            }
        }
    }

    fn send_mouse_to_active_terminal(&mut self, mouse: crossterm::event::MouseEvent) {
        if let Some(terminal) = self.active_terminal_mut() {
            let (mode, encoding) = terminal.mouse_protocol();
            if let Some(bytes) = mouse_event_to_bytes(mouse, mode, encoding) {
                terminal.write_bytes(&bytes);
            }
        }
    }

    fn ensure_active_workspace_tabs(&mut self, rows: u16, cols: u16) -> Result<(), String> {
        let Some(workspace) = self.active_workspace.clone() else {
            return Ok(());
        };
        if self.tabs.get(&workspace).map(|tabs| tabs.tabs.is_empty()).unwrap_or(true) {
            let name = self.spawn_tab(rows, cols, None)?;
            self.set_output(format!("Opened tab: {name}"));
        }
        Ok(())
    }

    fn create_tab_for_active(
        &mut self,
        rows: u16,
        cols: u16,
        name: Option<String>,
    ) -> Result<String, String> {
        if self.active_workspace.is_none() {
            return Err("No active workspace.".to_string());
        }
        self.spawn_tab(rows, cols, name)
    }

    fn spawn_tab(&mut self, rows: u16, cols: u16, name: Option<String>) -> Result<String, String> {
        let workspace = self
            .active_workspace
            .clone()
            .ok_or_else(|| "No active workspace.".to_string())?;
        let shell = self
            .config
            .terminal
            .command
            .clone()
            .unwrap_or_else(default_shell);
        let args = self.config.terminal.args.clone();
        let session = TerminalSession::spawn(
            self.terminal_seq,
            &shell,
            &args,
            &self.cwd,
            rows,
            cols,
            self.event_tx.clone(),
        )
        .map_err(|err| format!("Failed to start shell: {err}"))?;
        self.terminal_seq = self.terminal_seq.wrapping_add(1);

        let desired_name = match name {
            Some(name) => {
                let name = name.trim().to_string();
                self.validate_tab_name(&name)?;
                Some(name)
            }
            None => None,
        };

        let tabs = self.tabs.entry(workspace).or_insert_with(WorkspaceTabs::new);
        let name = match desired_name {
            Some(name) => {
                if tabs.tabs.iter().any(|tab| tab.name == name) {
                    return Err(format!("Tab '{name}' already exists."));
                }
                name
            }
            None => {
                let name = format!("tab-{}", tabs.next_index);
                tabs.next_index += 1;
                name
            }
        };
        tabs.tabs.push(WorkspaceTab {
            id: session.id(),
            name: name.clone(),
            terminal: session,
        });
        tabs.active = tabs.tabs.len().saturating_sub(1);
        Ok(name)
    }

    fn tab_next(&mut self) {
        let Some(workspace) = self.active_workspace.as_deref() else {
            return;
        };
        if let Some(tabs) = self.tabs.get_mut(workspace) {
            if tabs.tabs.is_empty() {
                return;
            }
            tabs.active = (tabs.active + 1) % tabs.tabs.len();
        }
    }

    fn tab_prev(&mut self) {
        let Some(workspace) = self.active_workspace.as_deref() else {
            return;
        };
        if let Some(tabs) = self.tabs.get_mut(workspace) {
            if tabs.tabs.is_empty() {
                return;
            }
            if tabs.active == 0 {
                tabs.active = tabs.tabs.len().saturating_sub(1);
            } else {
                tabs.active -= 1;
            }
        }
    }

    fn tab_select(&mut self, index: usize) {
        let Some(workspace) = self.active_workspace.as_deref() else {
            return;
        };
        if let Some(tabs) = self.tabs.get_mut(workspace) {
            if index < tabs.tabs.len() {
                tabs.active = index;
            }
        }
    }

    fn tab_select_by_arg(&mut self, arg: &str) {
        if let Ok(index) = arg.parse::<usize>() {
            if index == 0 {
                self.set_output("Tab index starts at 1.".to_string());
            } else {
                self.tab_select(index - 1);
            }
            return;
        }

        let Some(workspace) = self.active_workspace.as_deref() else {
            self.set_output("No active workspace.".to_string());
            return;
        };
        if let Some(tabs) = self.tabs.get_mut(workspace) {
            if let Some(index) = tabs.tabs.iter().position(|tab| tab.name == arg) {
                tabs.active = index;
            } else {
                self.set_output(format!("Tab '{arg}' not found."));
            }
        }
    }

    fn rename_active_tab(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        self.validate_tab_name(name)?;
        let workspace = self
            .active_workspace
            .as_deref()
            .ok_or_else(|| "No active workspace.".to_string())?;
        let tabs = self
            .tabs
            .get_mut(workspace)
            .ok_or_else(|| "No tabs for active workspace.".to_string())?;
        if tabs.tabs.iter().any(|tab| tab.name == name) {
            return Err(format!("Tab '{name}' already exists."));
        }
        let tab = tabs
            .tabs
            .get_mut(tabs.active)
            .ok_or_else(|| "No active tab.".to_string())?;
        tab.name = name.to_string();
        Ok(())
    }

    fn close_active_tab(&mut self) -> Result<String, String> {
        let workspace = self
            .active_workspace
            .as_deref()
            .ok_or_else(|| "No active workspace.".to_string())?;
        let tabs = self
            .tabs
            .get_mut(workspace)
            .ok_or_else(|| "No tabs for active workspace.".to_string())?;
        if tabs.tabs.len() <= 1 {
            return Err("Cannot close the last tab.".to_string());
        }
        let removed = tabs.tabs.remove(tabs.active);
        if tabs.active >= tabs.tabs.len() {
            tabs.active = tabs.tabs.len().saturating_sub(1);
        }
        Ok(format!("Closed tab: {}", removed.name))
    }

    fn validate_tab_name(&self, name: &str) -> Result<(), String> {
        if name.is_empty() {
            return Err("Tab name cannot be empty.".to_string());
        }
        if name.chars().any(|ch| ch.is_whitespace()) {
            return Err("Tab name cannot contain spaces.".to_string());
        }
        Ok(())
    }
}

fn inset_horizontal(area: Rect, padding: u16) -> Rect {
    if area.width <= padding * 2 {
        return area;
    }
    Rect {
        x: area.x + padding,
        width: area.width - padding * 2,
        ..area
    }
}


fn matches_command_open(key: KeyEvent) -> bool {
    key.code == KeyCode::Char(':')
}

fn spawn_input_thread(sender: Sender<AppEvent>) {
    std::thread::spawn(move || loop {
        match event::read() {
            Ok(Event::Key(key)) => {
                if sender.send(AppEvent::Input(key)).is_err() {
                    break;
                }
            }
            Ok(Event::Mouse(mouse)) => {
                if sender.send(AppEvent::Mouse(mouse)).is_err() {
                    break;
                }
            }
            Ok(Event::Resize(cols, rows)) => {
                if sender.send(AppEvent::Resize(rows, cols)).is_err() {
                    break;
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    });
}


fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);

    horizontal[1]
}

fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string())
    }
}
