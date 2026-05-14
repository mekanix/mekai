use std::sync::Arc;

use crate::kimi::config::Theme;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tokio::sync::mpsc;

use crate::kimi::error::Result;
use crate::kimi::llm::ChatProvider;
use crate::kimi::soul::{KimiSoul, SoulEvent};

pub struct Shell {
    pub lines: Vec<Line<'static>>,
    pub input: String,
    pub running: bool,
    pub yolo: bool,
    pub thinking: bool,
    pub plan_mode: bool,
    pub theme: Theme,
    pub current_response: String,
    pub show_thinking_stream: bool,
}

impl Default for Shell {
    fn default() -> Self {
        Self::new(Theme::default(), true)
    }
}

impl Shell {
    fn user_color(&self) -> Color {
        parse_color(self.theme.user_color.as_deref(), Color::Green)
    }

    fn assistant_color(&self) -> Color {
        parse_color(self.theme.assistant_color.as_deref(), Color::Blue)
    }

    fn error_color(&self) -> Color {
        parse_color(self.theme.error_color.as_deref(), Color::Red)
    }

    fn btw_color(&self) -> Color {
        parse_color(self.theme.btw_color.as_deref(), Color::Yellow)
    }

    fn border_color(&self) -> Color {
        parse_color(self.theme.border_style.as_deref(), Color::White)
    }
}

impl Shell {
    pub fn new(theme: Theme, show_thinking_stream: bool) -> Self {
        Self {
            lines: vec![
                Line::from("Welcome to Mekai! Type /help for commands."),
                Line::from(""),
            ],
            input: String::new(),
            running: true,
            yolo: true,
            thinking: false,
            plan_mode: false,
            theme,
            current_response: String::new(),
            show_thinking_stream,
        }
    }

    pub async fn run(
        &mut self,
        llm: Arc<dyn ChatProvider>,
        soul: &mut Option<KimiSoul>,
        initial_prompt: Option<String>,
    ) -> Result<i32> {
        let soul_ref = soul.as_mut().unwrap();
        self.plan_mode = soul_ref.plan_mode;
        self.thinking = soul_ref.thinking;

        // Spawn approval listener
        let approval = Arc::clone(&soul_ref.approval);
        let (approval_tx, mut approval_rx) = mpsc::channel::<(String, String)>(10);
        tokio::spawn(async move {
            loop {
                if let Some(req) = approval.next_pending().await {
                    let _ = approval_tx.send((req.tool_name, req.id)).await;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        });

        // Spawn keyboard reader
        let (key_tx, mut key_rx) = mpsc::channel::<event::KeyEvent>(100);
        tokio::spawn(async move {
            loop {
                if event::poll(std::time::Duration::from_millis(50)).unwrap_or(false)
                    && let Ok(Event::Key(key)) = event::read()
                    && key.kind == KeyEventKind::Press
                {
                    let _ = key_tx.send(key).await;
                }
            }
        });

        let mut terminal = setup_terminal()?;
        let result = self
            .run_loop(
                &mut terminal,
                soul,
                llm.clone(),
                &mut approval_rx,
                &mut key_rx,
                initial_prompt,
            )
            .await;
        restore_terminal(&mut terminal)?;
        result
    }

    async fn run_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
        soul: &mut Option<KimiSoul>,
        llm: Arc<dyn ChatProvider>,
        approval_rx: &mut mpsc::Receiver<(String, String)>,
        key_rx: &mut mpsc::Receiver<event::KeyEvent>,
        initial_prompt: Option<String>,
    ) -> Result<i32> {
        let mut stream_rx: Option<mpsc::Receiver<SoulEvent>> = None;
        let mut soul_return_rx: Option<mpsc::Receiver<KimiSoul>> = None;

        // Handle initial prompt if provided
        if let Some(prompt) = initial_prompt
            && !prompt.trim().is_empty()
        {
            self.lines.push(Line::from(vec![
                Span::styled("> ", Style::default().fg(self.user_color())),
                Span::raw(prompt.clone()),
            ]));
            self.thinking = true;
            self.current_response.clear();
            if let Some(mut s) = soul.take() {
                s.cancel.reset();
                s.approval.set_yolo(self.yolo).await;
                let (event_tx, event_rx) = mpsc::channel(1000);
                let (soul_tx, soul_rx) = mpsc::channel(1);
                stream_rx = Some(event_rx);
                soul_return_rx = Some(soul_rx);
                let llm2 = llm.clone();
                tokio::spawn(async move {
                    let _ = s.run_with_events(&prompt, llm2, event_tx).await;
                    let _ = soul_tx.send(s).await;
                });
            }
        }

        while self.running {
            let cancelled = soul.as_ref().is_some_and(|s| s.cancel.is_cancelled());
            if cancelled {
                self.running = false;
                break;
            }

            terminal.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(3), Constraint::Length(3)])
                    .split(f.area());

                let mut display_lines = self.lines.clone();
                if self.thinking && !self.current_response.is_empty() {
                    display_lines.push(Line::from(vec![
                        Span::styled("Assistant: ", Style::default().fg(self.assistant_color())),
                        Span::raw(self.current_response.clone()),
                    ]));
                }

                let border_style = Style::default().fg(self.border_color());
                let messages = Paragraph::new(display_lines)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(self.title())
                            .border_style(border_style),
                    )
                    .wrap(Wrap { trim: true });
                f.render_widget(messages, chunks[0]);

                let input_text = if self.thinking {
                    if self.plan_mode {
                        "Planning...".to_string()
                    } else {
                        format!("> {}", self.input)
                    }
                } else {
                    format!("> {}", self.input)
                };
                let input_widget = Paragraph::new(input_text).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Input")
                        .border_style(border_style),
                );
                f.render_widget(input_widget, chunks[1]);
            })?;

            tokio::select! {
                biased;

                // Handle stream events when a turn is running
                event = async {
                    if let Some(ref mut rx) = stream_rx {
                        rx.recv().await
                    } else {
                        std::future::pending().await
                    }
                } => {
                    match event {
                        Some(SoulEvent::Token(text)) => {
                            if self.show_thinking_stream {
                                self.current_response.push_str(&text);
                            }
                        }
                        Some(SoulEvent::ToolCall(call)) => {
                            self.lines.push(Line::from(vec![
                                Span::styled("Tool: ", Style::default().fg(Color::Cyan)),
                                Span::raw(format!("{}({})", call.function.name, call.function.arguments)),
                            ]));
                        }
                        Some(SoulEvent::ToolResult { output, .. }) => {
                            for line in output.lines() {
                                self.lines.push(Line::from(vec![
                                    Span::styled("  → ", Style::default().fg(Color::Cyan)),
                                    Span::raw(line.to_string()),
                                ]));
                            }
                        }
                        Some(SoulEvent::Plan(plan)) => {
                            self.current_response = plan.clone();
                        }
                        Some(SoulEvent::Done(response)) => {
                            self.thinking = false;
                            if !self.current_response.is_empty() {
                                for line in self.current_response.lines() {
                                    self.lines.push(Line::from(vec![
                                        Span::styled("Assistant: ", Style::default().fg(self.assistant_color())),
                                        Span::raw(line.to_string()),
                                    ]));
                                }
                            } else if !response.is_empty() {
                                for line in response.lines() {
                                    self.lines.push(Line::from(vec![
                                        Span::styled("Assistant: ", Style::default().fg(self.assistant_color())),
                                        Span::raw(line.to_string()),
                                    ]));
                                }
                            }
                            self.current_response.clear();
                            stream_rx = None;
                        }
                        Some(SoulEvent::Error(err)) => {
                            self.thinking = false;
                            self.lines.push(Line::from(vec![
                                Span::styled("Error: ", Style::default().fg(self.error_color())),
                                Span::raw(err),
                            ]));
                            self.current_response.clear();
                            stream_rx = None;
                        }
                        Some(SoulEvent::ApprovalNeeded { tool_name, .. }) => {
                            self.lines.push(Line::from(vec![
                                Span::styled("Approval needed: ", Style::default().fg(Color::Yellow)),
                                Span::raw(tool_name),
                            ]));
                        }
                        None => {
                            // Sender dropped without Done/Error — task panicked or hung
                            if self.thinking {
                                self.thinking = false;
                                self.lines.push(Line::from(vec![
                                    Span::styled("Error: ", Style::default().fg(self.error_color())),
                                    Span::raw("Soul task ended unexpectedly".to_string()),
                                ]));
                            }
                            stream_rx = None;
                        }
                    }
                }

                // Soul returned from spawned task
                returned_soul = async {
                    if let Some(ref mut rx) = soul_return_rx {
                        rx.recv().await
                    } else {
                        std::future::pending().await
                    }
                } => {
                    if let Some(s) = returned_soul {
                        *soul = Some(s);
                    }
                    soul_return_rx = None;
                }

                // Handle keyboard input
                Some(key) = key_rx.recv() => {
                    match key.code {
                        KeyCode::Char('c')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            if self.thinking {
                                if let Some(s) = soul {
                                    s.cancel.cancel();
                                }
                                self.lines.push(Line::from("Cancelling..."));
                            } else {
                                self.running = false;
                            }
                        }
                        KeyCode::Enter => {
                            if self.thinking {
                                continue;
                            }
                            let input = std::mem::take(&mut self.input);
                            if input.starts_with('/') {
                                if let Some(s) = soul {
                                    self.handle_slash_command(&input, s, llm.clone()).await?;
                                }
                            } else if !input.trim().is_empty() {
                                self.lines.push(Line::from(vec![
                                    Span::styled("> ", Style::default().fg(self.user_color())),
                                    Span::raw(input.clone()),
                                ]));
                                self.thinking = true;
                                self.current_response.clear();
                                if let Some(mut s) = soul.take() {
                                    s.cancel.reset();
                                    s.approval.set_yolo(self.yolo).await;
                                    let (event_tx, event_rx) = mpsc::channel(1000);
                                    let (soul_tx, soul_rx) = mpsc::channel(1);
                                    stream_rx = Some(event_rx);
                                    soul_return_rx = Some(soul_rx);
                                    let input = input.clone();
                                    let llm2 = llm.clone();
                                    tokio::spawn(async move {
                                        let _ = s.run_with_events(&input, llm2, event_tx).await;
                                        let _ = soul_tx.send(s).await;
                                    });
                                }
                            }
                        }
                        KeyCode::Char(c) if !self.thinking => {
                            self.input.push(c);
                        }
                        KeyCode::Backspace if !self.thinking => {
                            self.input.pop();
                        }
                        KeyCode::Esc => {
                            self.running = false;
                        }
                        _ => {}
                    }
                }

                // Handle approval requests
                Some((tool_name, req_id)) = approval_rx.recv() => {
                    if let Some(s) = soul {
                        self.show_approval_prompt(&tool_name, &req_id, s).await?;
                    }
                }

                else => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                }
            }
        }

        Ok(0)
    }

    fn title(&self) -> String {
        let mut title = "Mekai".to_string();
        if self.plan_mode {
            title.push_str(" [PLAN]");
        }
        if !self.yolo {
            title.push_str(" [SAFE]");
        }
        if self.thinking {
            title.push_str(" [STREAMING]");
        }
        title
    }

    async fn show_approval_prompt(
        &mut self,
        tool_name: &str,
        req_id: &str,
        soul: &mut KimiSoul,
    ) -> Result<()> {
        // Leave alternate screen and disable raw mode for terminal prompt
        let mut stdout = std::io::stdout();
        crossterm::execute!(stdout, crossterm::terminal::LeaveAlternateScreen)?;
        crossterm::terminal::disable_raw_mode()?;
        println!("\nApproval request for tool: {tool_name}");
        print!("Approve? (y/n): ");
        use std::io::Write;
        stdout.flush()?;
        let mut buffer = String::new();
        std::io::stdin().read_line(&mut buffer)?;
        let approved = buffer.trim().to_lowercase().starts_with('y');
        let _ = soul.approval.approve(req_id, approved, None).await;
        // Re-enter alternate screen and raw mode
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;
        Ok(())
    }

    async fn handle_slash_command(
        &mut self,
        cmd: &str,
        soul: &mut KimiSoul,
        llm: Arc<dyn ChatProvider>,
    ) -> Result<()> {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        match parts.first().copied() {
            Some("/quit") | Some("/q") => self.running = false,
            Some("/clear") => {
                self.lines.clear();
                self.lines.push(Line::from("Screen cleared."));
            }
            Some("/yolo") => {
                self.yolo = true;
                self.lines.push(Line::from("Yolo mode ON (auto-approve)."));
            }
            Some("/safe") => {
                self.yolo = false;
                self.lines
                    .push(Line::from("Safe mode ON (approvals required)."));
            }
            Some("/tools") => {
                self.lines.push(Line::from("Available tools:"));
                for name in soul.tools.keys() {
                    self.lines.push(Line::from(format!("  - {name}")));
                }
            }
            Some("/plan") => {
                soul.plan_mode = !soul.plan_mode;
                self.plan_mode = soul.plan_mode;
                if soul.plan_mode {
                    self.lines.push(Line::from("Plan mode ON."));
                } else {
                    self.lines.push(Line::from("Plan mode OFF."));
                }
            }
            Some("/fork") => {
                if let Some(ref session) = soul.session {
                    match session.fork(None).await {
                        Ok(new_session) => {
                            soul.session = Some(new_session);
                            self.lines.push(Line::from(format!(
                                "Forked to new session: {}",
                                soul.session.as_ref().unwrap().id
                            )));
                        }
                        Err(e) => {
                            self.lines.push(Line::from(format!("Fork failed: {e}")));
                        }
                    }
                } else {
                    self.lines.push(Line::from("No active session to fork."));
                }
            }
            Some("/undo") => {
                if soul.context.messages.len() > 1 {
                    // Remove last user + assistant pair
                    soul.context.messages.pop();
                    while let Some(last) = soul.context.messages.last() {
                        if last.role == "assistant" || last.role == "tool" {
                            soul.context.messages.pop();
                        } else {
                            break;
                        }
                    }
                    self.lines.push(Line::from("Last turn undone."));
                } else {
                    self.lines.push(Line::from("Nothing to undo."));
                }
            }
            Some("/checkpoint") => {
                let desc = parts.get(1..).map(|p| p.join(" ")).unwrap_or_default();
                let id = soul.checkpoint(if desc.is_empty() {
                    "manual checkpoint"
                } else {
                    &desc
                });
                self.lines
                    .push(Line::from(format!("Checkpoint created: {id}")));
            }
            Some("/rollback") => {
                let id = parts.get(1).copied().unwrap_or("");
                if id.is_empty() {
                    self.lines
                        .push(Line::from("Usage: /rollback <checkpoint_id>"));
                } else if soul.rollback(id) {
                    self.lines
                        .push(Line::from(format!("Rolled back to checkpoint: {id}")));
                } else {
                    self.lines
                        .push(Line::from(format!("Checkpoint not found: {id}")));
                }
            }
            Some("/tasks") => {
                let manager = crate::kimi::background::manager::BackgroundTaskManager::new(
                    4,
                    std::time::Duration::from_secs(900),
                    false,
                );
                let tasks = manager.list_tasks().await;
                if tasks.is_empty() {
                    self.lines.push(Line::from("No background tasks."));
                } else {
                    self.lines.push(Line::from("Background tasks:"));
                    for task in tasks {
                        self.lines.push(Line::from(format!(
                            "  {} - {:?}",
                            &task.id[..8],
                            task.status
                        )));
                    }
                }
            }
            Some("/btw") => {
                let question = parts.get(1..).map(|p| p.join(" ")).unwrap_or_default();
                if question.is_empty() {
                    self.lines.push(Line::from("Usage: /btw <question>"));
                } else {
                    self.lines.push(Line::from(vec![
                        Span::styled("BTW: ", Style::default().fg(self.btw_color())),
                        Span::raw(question.clone()),
                    ]));
                    let _ = soul.btw_tx.try_send(question);
                }
            }
            Some("/execute") => {
                if soul.pending_plan.is_some() {
                    self.thinking = true;
                    let result = soul.run("/execute", llm).await;
                    self.thinking = false;
                    match result {
                        Ok(response) => {
                            for line in response.lines() {
                                self.lines.push(Line::from(vec![
                                    Span::styled(
                                        "Assistant: ",
                                        Style::default().fg(self.assistant_color()),
                                    ),
                                    Span::raw(line.to_string()),
                                ]));
                            }
                        }
                        Err(e) => {
                            self.lines.push(Line::from(vec![
                                Span::styled("Error: ", Style::default().fg(self.error_color())),
                                Span::raw(e.to_string()),
                            ]));
                        }
                    }
                } else {
                    self.lines.push(Line::from("No pending plan to execute."));
                }
            }
            Some("/subagents") => {
                let store = soul.subagent_store.read().await;
                let agents = store.list();
                if agents.is_empty() {
                    self.lines.push(Line::from("No subagents recorded."));
                } else {
                    self.lines.push(Line::from("Subagents:"));
                    for agent in agents {
                        self.lines.push(Line::from(format!(
                            "  {} - {} [{}]",
                            &agent.id[..8.min(agent.id.len())],
                            agent.agent_type,
                            agent.status
                        )));
                    }
                }
            }
            Some("/notifications") => {
                if parts.get(1) == Some(&"clear") {
                    soul.notifications.clear().await;
                    self.lines.push(Line::from("Notifications cleared."));
                } else {
                    let notes = soul.notifications.list().await;
                    if notes.is_empty() {
                        self.lines.push(Line::from("No notifications."));
                    } else {
                        self.lines.push(Line::from("Notifications:"));
                        for note in notes.iter().rev().take(10) {
                            self.lines.push(Line::from(format!(
                                "  [{}] {:?} - {}",
                                note.source, note.level, note.message
                            )));
                        }
                    }
                }
            }
            Some("/todo") => {
                if let Some(sub) = parts.get(1).copied() {
                    match sub {
                        "add" => {
                            let text = parts.get(2..).map(|p| p.join(" ")).unwrap_or_default();
                            if text.is_empty() {
                                self.lines.push(Line::from("Usage: /todo add <text>"));
                            } else if let Some(ref mut session) = soul.session {
                                let id = uuid::Uuid::new_v4().to_string();
                                session.state.todos.push(crate::kimi::session_state::Todo {
                                    id: id.clone(),
                                    text,
                                    done: false,
                                });
                                let _ = session.save_state();
                                self.lines.push(Line::from(format!("Todo added: {id}")));
                            } else {
                                self.lines.push(Line::from("No active session."));
                            }
                        }
                        "list" => {
                            if let Some(ref session) = soul.session {
                                if session.state.todos.is_empty() {
                                    self.lines.push(Line::from("No todos."));
                                } else {
                                    self.lines.push(Line::from("Todos:"));
                                    for todo in &session.state.todos {
                                        let mark = if todo.done { "[x]" } else { "[ ]" };
                                        self.lines.push(Line::from(format!(
                                            "  {mark} {} - {}",
                                            &todo.id[..8.min(todo.id.len())],
                                            todo.text
                                        )));
                                    }
                                }
                            } else {
                                self.lines.push(Line::from("No active session."));
                            }
                        }
                        "done" => {
                            let id = parts.get(2).copied().unwrap_or("");
                            if id.is_empty() {
                                self.lines.push(Line::from("Usage: /todo done <id>"));
                            } else if let Some(ref mut session) = soul.session {
                                let found = session
                                    .state
                                    .todos
                                    .iter_mut()
                                    .find(|t| t.id.starts_with(id))
                                    .map(|t| {
                                        t.done = true;
                                        t.id.clone()
                                    });
                                if let Some(todo_id) = found {
                                    let _ = session.save_state();
                                    self.lines
                                        .push(Line::from(format!("Todo marked done: {todo_id}")));
                                } else {
                                    self.lines.push(Line::from(format!("Todo not found: {id}")));
                                }
                            } else {
                                self.lines.push(Line::from("No active session."));
                            }
                        }
                        "remove" => {
                            let id = parts.get(2).copied().unwrap_or("");
                            if id.is_empty() {
                                self.lines.push(Line::from("Usage: /todo remove <id>"));
                            } else if let Some(ref mut session) = soul.session {
                                let before = session.state.todos.len();
                                session.state.todos.retain(|t| !t.id.starts_with(id));
                                let removed = session.state.todos.len() < before;
                                if removed {
                                    let _ = session.save_state();
                                    self.lines.push(Line::from("Todo removed."));
                                } else {
                                    self.lines.push(Line::from(format!("Todo not found: {id}")));
                                }
                            } else {
                                self.lines.push(Line::from("No active session."));
                            }
                        }
                        _ => self
                            .lines
                            .push(Line::from("Usage: /todo [add|list|done|remove]")),
                    }
                } else {
                    self.lines
                        .push(Line::from("Usage: /todo [add|list|done|remove]"));
                }
            }
            Some("/archive") => {
                if let Some(ref session) = soul.session {
                    if session.state.archive.is_empty() {
                        self.lines.push(Line::from("No archived turns."));
                    } else {
                        self.lines.push(Line::from("Archived turns:"));
                        for turn in &session.state.archive {
                            self.lines.push(Line::from(format!(
                                "  Turn {}: {}",
                                turn.turn, turn.summary
                            )));
                        }
                    }
                } else {
                    self.lines.push(Line::from("No active session."));
                }
            }
            Some("/data") => {
                if let Some(sub) = parts.get(1).copied() {
                    match sub {
                        "set" => {
                            let key = parts.get(2).copied().unwrap_or("");
                            let value = parts.get(3..).map(|p| p.join(" ")).unwrap_or_default();
                            if key.is_empty() {
                                self.lines
                                    .push(Line::from("Usage: /data set <key> <value>"));
                            } else if let Some(ref mut session) = soul.session {
                                session
                                    .state
                                    .custom_data
                                    .insert(key.to_string(), serde_json::Value::String(value));
                                let _ = session.save_state();
                                self.lines.push(Line::from(format!("Data set: {key}")));
                            } else {
                                self.lines.push(Line::from("No active session."));
                            }
                        }
                        "get" => {
                            let key = parts.get(2).copied().unwrap_or("");
                            if key.is_empty() {
                                self.lines.push(Line::from("Usage: /data get <key>"));
                            } else if let Some(ref session) = soul.session {
                                match session.state.custom_data.get(key) {
                                    Some(v) => self.lines.push(Line::from(format!("{key} = {v}"))),
                                    None => {
                                        self.lines.push(Line::from(format!("Key not found: {key}")))
                                    }
                                }
                            } else {
                                self.lines.push(Line::from("No active session."));
                            }
                        }
                        _ => self.lines.push(Line::from("Usage: /data [set|get]")),
                    }
                } else {
                    self.lines.push(Line::from("Usage: /data [set|get]"));
                }
            }
            Some("/help") => {
                self.lines.push(Line::from("Commands:"));
                self.lines.push(Line::from("  /quit, /q   - Exit"));
                self.lines.push(Line::from("  /clear      - Clear screen"));
                self.lines
                    .push(Line::from("  /yolo       - Auto-approve all tools"));
                self.lines
                    .push(Line::from("  /safe       - Require approvals"));
                self.lines
                    .push(Line::from("  /plan       - Toggle plan mode"));
                self.lines
                    .push(Line::from("  /tools      - List available tools"));
                self.lines
                    .push(Line::from("  /fork       - Fork current session"));
                self.lines
                    .push(Line::from("  /undo       - Undo last turn"));
                self.lines
                    .push(Line::from("  /checkpoint - Save a checkpoint"));
                self.lines
                    .push(Line::from("  /rollback   - Rollback to a checkpoint"));
                self.lines
                    .push(Line::from("  /tasks      - List background tasks"));
                self.lines
                    .push(Line::from("  /subagents  - List subagent activity"));
                self.lines.push(Line::from(
                    "  /notifications [clear] - Show/clear notifications",
                ));
                self.lines.push(Line::from(
                    "  /todo       - Manage todos (add/list/done/remove)",
                ));
                self.lines
                    .push(Line::from("  /archive    - Show archived turns"));
                self.lines
                    .push(Line::from("  /data       - Get/set custom data"));
                self.lines
                    .push(Line::from("  /btw        - Ask a side question"));
                self.lines
                    .push(Line::from("  /execute    - Execute pending plan"));
                self.lines
                    .push(Line::from("  /help       - Show this help"));
            }
            _ => self
                .lines
                .push(Line::from(format!("Unknown command: {cmd}"))),
        }
        Ok(())
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

fn parse_color(name: Option<&str>, default: Color) -> Color {
    match name {
        Some("black") => Color::Black,
        Some("red") => Color::Red,
        Some("green") => Color::Green,
        Some("yellow") => Color::Yellow,
        Some("blue") => Color::Blue,
        Some("magenta") => Color::Magenta,
        Some("cyan") => Color::Cyan,
        Some("gray") | Some("grey") => Color::Gray,
        Some("dark_gray") | Some("dark_grey") => Color::DarkGray,
        Some("light_red") => Color::LightRed,
        Some("light_green") => Color::LightGreen,
        Some("light_yellow") => Color::LightYellow,
        Some("light_blue") => Color::LightBlue,
        Some("light_magenta") => Color::LightMagenta,
        Some("light_cyan") => Color::LightCyan,
        Some("white") => Color::White,
        _ => default,
    }
}
