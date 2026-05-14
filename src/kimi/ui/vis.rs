use std::path::PathBuf;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

use crate::kimi::error::Result;
use crate::kimi::wire::types::WireEvent;

pub struct Visualizer {
    sessions: Vec<SessionEntry>,
    selected: usize,
    events: Vec<WireEvent>,
    event_selected: usize,
    running: bool,
    focus: Focus,
}

enum Focus {
    Sessions,
    Events,
}

#[derive(Debug, Clone)]
struct SessionEntry {
    id: String,
    title: String,
    updated_at: String,
    path: PathBuf,
}

impl Default for Visualizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Visualizer {
    pub fn new() -> Self {
        Self {
            sessions: vec![],
            selected: 0,
            events: vec![],
            event_selected: 0,
            running: true,
            focus: Focus::Sessions,
        }
    }

    pub async fn run(&mut self) -> Result<i32> {
        self.load_sessions()?;

        let mut terminal = setup_terminal()?;
        let result = self.run_loop(&mut terminal).await;
        restore_terminal(&mut terminal)?;
        result
    }

    fn load_sessions(&mut self) -> Result<()> {
        let sessions_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("mekai")
            .join("sessions");

        if !sessions_dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(&sessions_dir)? {
            let entry = entry?;
            let session_json = entry.path().join("session.json");
            if session_json.exists()
                && let Ok(content) = std::fs::read_to_string(&session_json)
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(&content)
            {
                let id = value["id"].as_str().unwrap_or("unknown").to_string();
                let title = value["title"].as_str().unwrap_or("Untitled").to_string();
                let updated = value["updated_at"].as_str().unwrap_or("").to_string();
                self.sessions.push(SessionEntry {
                    id: id.clone(),
                    title,
                    updated_at: updated,
                    path: entry.path(),
                });
            }
        }

        self.sessions
            .sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(())
    }

    fn load_events(&mut self, session: &SessionEntry) -> Result<()> {
        self.events.clear();
        self.event_selected = 0;
        let wire_file = session.path.join("wire.jsonl");
        if !wire_file.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&wire_file)?;
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<WireEvent>(line) {
                self.events.push(event);
            }
        }
        Ok(())
    }

    async fn run_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<i32> {
        if !self.sessions.is_empty() {
            self.load_events(&self.sessions[self.selected].clone())?;
        }

        while self.running {
            terminal.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
                    .split(f.area());

                // Session list
                let session_items: Vec<ListItem> = self
                    .sessions
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        let style = if i == self.selected {
                            Style::default()
                                .fg(Color::Black)
                                .bg(Color::Cyan)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        };
                        ListItem::new(Line::from(vec![
                            Span::raw(format!("{} ", &s.id[..8.min(s.id.len())])),
                            Span::styled(&s.title, Style::default().fg(Color::Yellow)),
                        ]))
                        .style(style)
                    })
                    .collect();

                let session_list = List::new(session_items).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Sessions")
                        .border_style(match self.focus {
                            Focus::Sessions => Style::default().fg(Color::Cyan),
                            Focus::Events => Style::default(),
                        }),
                );
                f.render_widget(session_list, chunks[0]);

                // Events
                let event_lines: Vec<Line> = self
                    .events
                    .iter()
                    .enumerate()
                    .map(|(i, e)| {
                        let text = format_event(e);
                        let style = if i == self.event_selected {
                            Style::default().fg(Color::Black).bg(Color::Green)
                        } else {
                            Style::default()
                        };
                        Line::from(Span::styled(text, style))
                    })
                    .collect();

                let event_widget = Paragraph::new(event_lines)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!(
                                "Events ({})",
                                self.sessions.get(self.selected).map_or("", |s| &s.title)
                            ))
                            .border_style(match self.focus {
                                Focus::Sessions => Style::default(),
                                Focus::Events => Style::default().fg(Color::Cyan),
                            }),
                    )
                    .wrap(Wrap { trim: true });
                f.render_widget(event_widget, chunks[1]);
            })?;

            if event::poll(std::time::Duration::from_millis(50))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                match key.code {
                    KeyCode::Char('q') => self.running = false,
                    KeyCode::Tab => {
                        self.focus = match self.focus {
                            Focus::Sessions => Focus::Events,
                            Focus::Events => Focus::Sessions,
                        };
                    }
                    KeyCode::Up => match self.focus {
                        Focus::Sessions => {
                            if self.selected > 0 {
                                self.selected -= 1;
                                if let Ok(()) =
                                    self.load_events(&self.sessions[self.selected].clone())
                                {
                                }
                            }
                        }
                        Focus::Events => {
                            if self.event_selected > 0 {
                                self.event_selected -= 1;
                            }
                        }
                    },
                    KeyCode::Down => match self.focus {
                        Focus::Sessions => {
                            if self.selected + 1 < self.sessions.len() {
                                self.selected += 1;
                                if let Ok(()) =
                                    self.load_events(&self.sessions[self.selected].clone())
                                {
                                }
                            }
                        }
                        Focus::Events => {
                            if self.event_selected + 1 < self.events.len() {
                                self.event_selected += 1;
                            }
                        }
                    },
                    _ => {}
                }
            }
        }

        Ok(0)
    }
}

fn format_event(event: &WireEvent) -> String {
    match event {
        WireEvent::TurnBegin { turn, .. } => format!("[TurnBegin] turn={turn}"),
        WireEvent::StepBegin { step, .. } => format!("[StepBegin] step={step}"),
        WireEvent::StepInterrupted { step, reason, .. } => {
            format!("[StepInterrupted] step={step} reason={reason}")
        }
        WireEvent::TurnEnd { turn, .. } => format!("[TurnEnd] turn={turn}"),
        WireEvent::RalphTurnBegin { iteration, .. } => {
            format!("[RalphTurnBegin] iteration={iteration}")
        }
        WireEvent::RalphTurnEnd { iteration, .. } => {
            format!("[RalphTurnEnd] iteration={iteration}")
        }
        WireEvent::StatusUpdate { message, .. } => format!("[Status] {message}"),
        WireEvent::CompactionBegin { .. } => "[CompactionBegin]".to_string(),
        WireEvent::CompactionEnd { .. } => "[CompactionEnd]".to_string(),
        WireEvent::MCPLoadingBegin { .. } => "[MCPLoadingBegin]".to_string(),
        WireEvent::MCPLoadingEnd { .. } => "[MCPLoadingEnd]".to_string(),
        WireEvent::Notification { level, message, .. } => {
            format!("[Notification {level}] {message}")
        }
        WireEvent::SubagentEvent { subagent_id, .. } => {
            format!("[Subagent {subagent_id}] ...")
        }
        WireEvent::BtwBegin { question, .. } => format!("[BTW] {question}"),
        WireEvent::BtwEnd { answer, .. } => format!("[BTW Answer] {answer}"),
        WireEvent::PlanDisplay { plan, .. } => format!("[Plan] {plan}"),
        WireEvent::ContentPart { content, .. } => format!("[Content] {content}"),
        WireEvent::ToolCall { call, .. } => {
            format!(
                "[ToolCall] {}({})",
                call.function.name, call.function.arguments
            )
        }
        WireEvent::ToolResult {
            call_id, result, ..
        } => {
            format!(
                "[ToolResult] {} success={}",
                &call_id[..8.min(call_id.len())],
                result.success
            )
        }
        WireEvent::ApprovalRequest {
            request_id,
            tool_name,
            ..
        } => format!("[Approval] {tool_name} ({request_id})"),
        WireEvent::ApprovalResponse {
            request_id,
            approved,
            ..
        } => {
            format!(
                "[ApprovalResponse] {} approved={approved}",
                &request_id[..8.min(request_id.len())]
            )
        }
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen
    )?;
    crossterm::terminal::disable_raw_mode()?;
    Ok(())
}
