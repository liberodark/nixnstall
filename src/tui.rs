use crate::answers::Answers;
use crate::config::{Config, Question};
use crate::error::{NixstallError, Result};
use crate::probe::Disk;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout as UiLayout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use std::io::Stdout;
use tokio::sync::mpsc::UnboundedReceiver;

pub struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Warnings,
    Disk,
    Layout,
    Profile,
    Options,
    Hostname,
    Username,
    Password,
    Confirm,
}

impl Tui {
    pub fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut out = std::io::stdout();
        execute!(out, EnterAlternateScreen)?;
        let terminal = Terminal::new(CrosstermBackend::new(out))?;
        Ok(Self { terminal })
    }

    pub fn restore(&mut self) -> Result<()> {
        disable_raw_mode()?;
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)?;
        self.terminal.show_cursor()?;
        Ok(())
    }

    pub fn wizard(
        &mut self,
        config: &Config,
        disks: &[Disk],
        warnings: &[String],
    ) -> Result<(Answers, String)> {
        let layouts: Vec<String> = config.layout.keys().cloned().collect();
        let profiles: Vec<String> = config.profile.keys().cloned().collect();

        let mut step = if warnings.is_empty() {
            Step::Disk
        } else {
            Step::Warnings
        };
        let mut disk_idx = 0usize;
        let mut layout_idx = 0usize;
        let mut profile_idx = 0usize;
        let mut option_idx = 0usize;
        let mut options: std::collections::BTreeMap<String, serde_json::Value> = config
            .question
            .iter()
            .map(|q| (q.key().to_string(), q.default_value()))
            .collect();
        let mut hostname = config.installer.default_hostname.clone();
        let mut username = config.installer.default_username.clone();
        let mut password = String::new();
        let mut confirm = String::new();
        let mut confirming = false;
        let mut error: Option<String> = None;

        loop {
            let title = config.installer.title.clone();
            self.terminal.draw(|frame| {
                let area = frame.area();
                let chunks = UiLayout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3),
                        Constraint::Min(5),
                        Constraint::Length(3),
                    ])
                    .split(area);

                frame.render_widget(header(&title, step), chunks[0]);

                match step {
                    Step::Warnings => frame.render_widget(warn_panel(warnings), chunks[1]),
                    Step::Disk => {
                        let items: Vec<ListItem> =
                            disks.iter().map(|d| ListItem::new(d.label())).collect();
                        render_list(frame, chunks[1], "Target disk (will be ERASED)", items, disk_idx);
                    }
                    Step::Layout => {
                        let items: Vec<ListItem> = layouts
                            .iter()
                            .map(|k| {
                                let d = &config.layout[k].description;
                                ListItem::new(format!("{k:<8} {d}"))
                            })
                            .collect();
                        render_list(frame, chunks[1], "Filesystem", items, layout_idx);
                    }
                    Step::Profile => {
                        let items: Vec<ListItem> = profiles
                            .iter()
                            .map(|k| {
                                let d = &config.profile[k].description;
                                ListItem::new(format!("{k:<14} {d}"))
                            })
                            .collect();
                        render_list(frame, chunks[1], "Profile", items, profile_idx);
                    }
                    Step::Options => {
                        let items: Vec<ListItem> = config
                            .question
                            .iter()
                            .map(|q| {
                                let value = match options.get(q.key()) {
                                    Some(serde_json::Value::Bool(b)) => {
                                        if *b { "yes".to_string() } else { "no".to_string() }
                                    }
                                    Some(serde_json::Value::String(s)) => s.clone(),
                                    _ => String::new(),
                                };
                                ListItem::new(format!("{:<22} {}", q.prompt(), value))
                            })
                            .collect();
                        render_list(frame, chunks[1], "Options", items, option_idx);
                    }
                    Step::Hostname => frame.render_widget(input("Hostname", &hostname, false), chunks[1]),
                    Step::Username => frame.render_widget(input("Username", &username, false), chunks[1]),
                    Step::Password => {
                        let label = if confirming { "Confirm password" } else { "Password" };
                        let value = if confirming { &confirm } else { &password };
                        frame.render_widget(input(label, value, true), chunks[1]);
                    }
                    Step::Confirm => {
                        let profile = profiles.get(profile_idx).cloned().unwrap_or_default();
                        let summary = format!(
                            "Disk       {}\nFilesystem {}\nHostname   {}\nUser       {}\nProfile    {}\n\nEverything on the disk will be destroyed.",
                            disks[disk_idx].path, layouts[layout_idx], hostname, username, profile
                        );
                        frame.render_widget(
                            Paragraph::new(summary)
                                .block(Block::default().borders(Borders::ALL).title(" Review "))
                                .wrap(Wrap { trim: false }),
                            chunks[1],
                        );
                    }
                }

                let hint = if step == Step::Options {
                    config
                        .question
                        .get(option_idx)
                        .map(|q| q.help())
                        .filter(|h| !h.is_empty())
                } else {
                    None
                };
                frame.render_widget(footer(step, error.as_deref(), hint), chunks[2]);
            })?;

            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            error = None;

            match (step, key.code) {
                (_, KeyCode::Esc) => match step {
                    Step::Warnings | Step::Disk => return Err(NixstallError::Cancelled),
                    Step::Layout => step = Step::Disk,
                    Step::Profile => step = Step::Layout,
                    Step::Options => step = Step::Profile,
                    Step::Hostname => {
                        step = if config.question.is_empty() {
                            Step::Profile
                        } else {
                            Step::Options
                        }
                    }
                    Step::Username => step = Step::Hostname,
                    Step::Password => {
                        password.clear();
                        confirm.clear();
                        confirming = false;
                        step = Step::Username;
                    }
                    Step::Confirm => step = Step::Password,
                },
                (Step::Warnings, KeyCode::Enter) => step = Step::Disk,

                (Step::Disk, KeyCode::Up) => disk_idx = disk_idx.saturating_sub(1),
                (Step::Disk, KeyCode::Down) => {
                    disk_idx = (disk_idx + 1).min(disks.len().saturating_sub(1))
                }
                (Step::Disk, KeyCode::Enter) => step = Step::Layout,

                (Step::Layout, KeyCode::Up) => layout_idx = layout_idx.saturating_sub(1),
                (Step::Layout, KeyCode::Down) => {
                    layout_idx = (layout_idx + 1).min(layouts.len().saturating_sub(1))
                }
                (Step::Layout, KeyCode::Enter) => step = Step::Profile,

                (Step::Profile, KeyCode::Up) => profile_idx = profile_idx.saturating_sub(1),
                (Step::Profile, KeyCode::Down) => {
                    profile_idx = (profile_idx + 1).min(profiles.len().saturating_sub(1))
                }
                (Step::Profile, KeyCode::Enter) => {
                    step = if config.question.is_empty() {
                        Step::Hostname
                    } else {
                        Step::Options
                    }
                }

                (Step::Options, KeyCode::Up) => option_idx = option_idx.saturating_sub(1),
                (Step::Options, KeyCode::Down) => {
                    option_idx = (option_idx + 1).min(config.question.len().saturating_sub(1))
                }
                (Step::Options, KeyCode::Left)
                | (Step::Options, KeyCode::Right)
                | (Step::Options, KeyCode::Char(' ')) => {
                    if let Some(q) = config.question.get(option_idx) {
                        let next = match (q, options.get(q.key())) {
                            (Question::Bool { .. }, Some(serde_json::Value::Bool(b))) => {
                                serde_json::Value::Bool(!b)
                            }
                            (
                                Question::Choice { choices, .. },
                                Some(serde_json::Value::String(current)),
                            ) => {
                                let at = choices.iter().position(|c| c == current).unwrap_or(0);
                                let step = if key.code == KeyCode::Left {
                                    choices.len().saturating_sub(1)
                                } else {
                                    1
                                };
                                let next = (at + step) % choices.len().max(1);
                                serde_json::Value::String(
                                    choices.get(next).cloned().unwrap_or_default(),
                                )
                            }
                            _ => q.default_value(),
                        };
                        options.insert(q.key().to_string(), next);
                    }
                }
                (Step::Options, KeyCode::Enter) => {
                    if let Some(serde_json::Value::String(km)) = config
                        .installer
                        .apply_keymap_from
                        .as_ref()
                        .and_then(|key| options.get(key))
                    {
                        let _ = std::process::Command::new("loadkeys").arg(km).status();
                    }
                    step = Step::Hostname;
                }

                (Step::Hostname, KeyCode::Char(c)) => hostname.push(c),
                (Step::Hostname, KeyCode::Backspace) => {
                    hostname.pop();
                }
                (Step::Hostname, KeyCode::Enter) => {
                    if hostname.is_empty() {
                        error = Some("Hostname cannot be empty".into());
                    } else {
                        step = Step::Username;
                    }
                }

                (Step::Username, KeyCode::Char(c)) => username.push(c),
                (Step::Username, KeyCode::Backspace) => {
                    username.pop();
                }
                (Step::Username, KeyCode::Enter) => {
                    if username.is_empty() {
                        error = Some("Username cannot be empty".into());
                    } else {
                        step = Step::Password;
                    }
                }

                (Step::Password, KeyCode::Char(c)) => {
                    if confirming {
                        confirm.push(c);
                    } else {
                        password.push(c);
                    }
                }
                (Step::Password, KeyCode::Backspace) => {
                    if confirming {
                        confirm.pop();
                    } else {
                        password.pop();
                    }
                }
                (Step::Password, KeyCode::Enter) => {
                    if password.is_empty() {
                        error = Some("Password cannot be empty".into());
                    } else if !confirming {
                        confirming = true;
                    } else if confirm != password {
                        error = Some("Passwords do not match".into());
                        confirm.clear();
                    } else {
                        step = Step::Confirm;
                    }
                }

                (Step::Confirm, KeyCode::Enter) => {
                    let answers = Answers {
                        options: options.clone(),
                        device: disks[disk_idx].path.clone(),
                        layout: layouts[layout_idx].clone(),
                        hostname: hostname.clone(),
                        username: username.clone(),
                        hashed_password: String::new(),
                        profile: profiles.get(profile_idx).cloned().unwrap_or_default(),
                    };
                    return Ok((answers, password));
                }
                _ => {}
            }
        }
    }

    pub fn ask_reboot(&mut self, message: &str, countdown: u64) -> Result<bool> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(countdown);
        let mut cancelled = false;

        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if !cancelled && left.is_zero() {
                return Ok(true);
            }

            let footer = if cancelled {
                "r reboot   q leave the installer".to_string()
            } else {
                format!(
                    "Rebooting in {}s   r reboot now   q stay here",
                    left.as_secs() + 1
                )
            };

            self.terminal.draw(|frame| {
                let area = frame.area();
                let chunks = UiLayout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(5), Constraint::Length(3)])
                    .split(area);
                frame.render_widget(
                    Paragraph::new(message.to_string())
                        .block(Block::default().borders(Borders::ALL).title(" Done "))
                        .wrap(Wrap { trim: false }),
                    chunks[0],
                );
                frame.render_widget(
                    Paragraph::new(footer).block(Block::default().borders(Borders::ALL)),
                    chunks[1],
                );
            })?;

            // Poll rather than block, so the countdown keeps ticking.
            if !event::poll(std::time::Duration::from_millis(200))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('r') | KeyCode::Char('R') | KeyCode::Enter => return Ok(true),
                KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => return Ok(false),
                // any other key stops the countdown without deciding
                _ => cancelled = true,
            }
        }
    }

    pub async fn progress(
        &mut self,
        rx: &mut UnboundedReceiver<String>,
        title: &str,
    ) -> Result<()> {
        let mut lines: Vec<String> = Vec::new();
        while let Some(line) = rx.recv().await {
            lines.push(line);
            let visible = lines.len().saturating_sub(40);
            let body: Vec<Line> = lines[visible..]
                .iter()
                .map(|l| Line::from(l.as_str()))
                .collect();
            self.terminal.draw(|frame| {
                frame.render_widget(
                    Paragraph::new(body)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(format!(" {title} ")),
                        )
                        .wrap(Wrap { trim: false }),
                    frame.area(),
                );
            })?;
        }
        Ok(())
    }
}

fn header(title: &str, step: Step) -> Paragraph<'static> {
    let name = match step {
        Step::Warnings => "Preflight",
        Step::Disk => "Disk",
        Step::Layout => "Filesystem",
        Step::Profile => "Profile",
        Step::Options => "Options",
        Step::Hostname => "Hostname",
        Step::Username => "User",
        Step::Password => "Password",
        Step::Confirm => "Review",
    };
    Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {title} "),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("— {name}")),
    ]))
    .block(Block::default().borders(Borders::ALL))
}

fn footer(step: Step, error: Option<&str>, hint: Option<&str>) -> Paragraph<'static> {
    if let Some(err) = error {
        return Paragraph::new(err.to_string())
            .style(Style::default().fg(Color::Red))
            .block(Block::default().borders(Borders::ALL));
    }
    if let Some(help) = hint {
        return Paragraph::new(help.to_string()).block(Block::default().borders(Borders::ALL));
    }
    let hint = match step {
        Step::Disk | Step::Layout | Step::Profile => "Up/Down select   Enter continue   Esc back",
        Step::Confirm => "Enter INSTALL   Esc back",
        _ => "Type   Enter continue   Esc back",
    };
    Paragraph::new(hint).block(Block::default().borders(Borders::ALL))
}

fn warn_panel(warnings: &[String]) -> Paragraph<'static> {
    let mut text = vec![Line::from("Some checks did not pass:"), Line::from("")];
    for w in warnings {
        text.push(Line::from(format!("  - {w}")));
    }
    text.push(Line::from(""));
    text.push(Line::from("Press Enter to continue anyway, Esc to abort."));
    Paragraph::new(text)
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().borders(Borders::ALL).title(" Preflight "))
        .wrap(Wrap { trim: false })
}

fn input(label: &str, value: &str, secret: bool) -> Paragraph<'static> {
    let shown = if secret {
        "*".repeat(value.chars().count())
    } else {
        value.to_string()
    };
    Paragraph::new(format!("\n  {shown}_")).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" {label} ")),
    )
}

fn render_list(
    frame: &mut ratatui::Frame,
    area: Rect,
    title: &str,
    items: Vec<ListItem<'static>>,
    selected: usize,
) {
    let mut state = ListState::default();
    state.select(Some(selected));
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {title} ")),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, area, &mut state);
}
