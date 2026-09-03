use std::{
    collections::VecDeque,
    io::{self, IsTerminal},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use crossterm::{
    cursor::{Hide, Show},
    event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use tokio::{sync::mpsc, time::interval};

use crate::{
    cli::StreamArgs,
    realtime::{self, RealtimeUpdate},
};

const LOG_LIMIT: usize = 500;

#[derive(Clone, Copy, Debug)]
enum ConnectionState {
    Connecting,
    Live,
    Reconnecting,
    Ended,
}

impl ConnectionState {
    const fn label(self) -> &'static str {
        match self {
            Self::Connecting => "CONNECTING",
            Self::Live => "LIVE",
            Self::Reconnecting => "RECONNECTING",
            Self::Ended => "ENDED",
        }
    }

    const fn color(self) -> Color {
        match self {
            Self::Connecting | Self::Reconnecting => Color::Yellow,
            Self::Live => Color::Green,
            Self::Ended => Color::Red,
        }
    }
}

struct TerminalRestore;

impl Drop for TerminalRestore {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
    }
}

/// Runs the realtime stream inside a full-screen terminal monitor.
///
/// # Errors
///
/// Returns an error when no interactive terminal exists or terminal setup fails.
pub async fn run(args: StreamArgs) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("the live monitor requires an interactive terminal; use `hector stream` for JSONL")
    }

    enable_raw_mode().context("failed to enable terminal raw mode")?;
    execute!(io::stdout(), EnterAlternateScreen, Hide)
        .context("failed to enter the alternate terminal screen")?;
    let _restore = TerminalRestore;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("failed to initialize live terminal")?;
    terminal.clear()?;

    let (sender, mut receiver) = mpsc::unbounded_channel();
    let error_sender = sender.clone();
    let stream_args = args.clone();
    let stream_task = tokio::spawn(async move {
        if let Err(error) = realtime::stream_updates(&stream_args, sender).await {
            let _ = error_sender.send(RealtimeUpdate::Raw(format!("ERROR {error:#}")));
        }
    });

    let mut input = EventStream::new();
    let mut ticker = interval(Duration::from_millis(100));
    let mut state = ConnectionState::Connecting;
    let mut logs = VecDeque::new();
    let mut paused = false;
    let mut last_update = "waiting".to_owned();

    loop {
        terminal.draw(|frame| {
            let areas = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Min(4),
                    Constraint::Length(2),
                ])
                .split(frame.area());

            let header = Paragraph::new(Line::from(vec![
                Span::styled(
                    " HECTOR ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("// REALTIME", Style::default().fg(Color::DarkGray)),
                Span::raw("     "),
                Span::styled(
                    format!("● {}", state.label()),
                    Style::default()
                        .fg(state.color())
                        .add_modifier(Modifier::BOLD),
                ),
            ]))
            .block(Block::default().borders(Borders::BOTTOM));
            frame.render_widget(header, areas[0]);

            let topic = args.topic.join("  ·  ");
            let topic = Paragraph::new(Line::from(vec![
                Span::styled("TOPICS  ", Style::default().fg(Color::DarkGray)),
                Span::styled(topic, Style::default().fg(Color::Cyan)),
            ]))
            .wrap(Wrap { trim: true });
            frame.render_widget(topic, areas[1]);

            let visible_height = usize::from(areas[2].height.saturating_sub(2));
            let start = logs.len().saturating_sub(visible_height);
            let lines = logs
                .iter()
                .skip(start)
                .map(|line| Line::from(line.as_str()))
                .collect::<Vec<_>>();
            let title = if paused {
                " MARKET FEED · PAUSED "
            } else {
                " MARKET FEED "
            };
            let feed = Paragraph::new(lines)
                .style(Style::default().fg(Color::White))
                .block(
                    Block::default()
                        .title(Span::styled(title, Style::default().fg(Color::Cyan)))
                        .borders(Borders::TOP),
                )
                .wrap(Wrap { trim: false });
            frame.render_widget(feed, areas[2]);

            let footer = Paragraph::new(Line::from(vec![
                Span::styled(
                    " q/esc ",
                    Style::default().fg(Color::Black).bg(Color::White),
                ),
                Span::raw(" quit   "),
                Span::styled(
                    " space ",
                    Style::default().fg(Color::Black).bg(Color::White),
                ),
                Span::raw(format!(" pause   last update {last_update}")),
            ]));
            frame.render_widget(footer, areas[3]);
        })?;

        tokio::select! {
            update = receiver.recv() => {
                let Some(update) = update else {
                    state = ConnectionState::Ended;
                    break;
                };
                match update {
                    RealtimeUpdate::Connecting => state = ConnectionState::Connecting,
                    RealtimeUpdate::Connected => {
                        state = ConnectionState::Live;
                        push_log(&mut logs, "CONNECTED · subscriptions active".to_owned());
                    }
                    RealtimeUpdate::Reconnecting { delay_ms, error } => {
                        state = ConnectionState::Reconnecting;
                        push_log(&mut logs, format!("RECONNECTING in {delay_ms} ms · {error}"));
                    }
                    RealtimeUpdate::Raw(frame) if !paused => {
                        push_log(&mut logs, frame);
                        last_update = now_label();
                    }
                    RealtimeUpdate::Event(event) if !paused => {
                        let line = serde_json::to_string(&event)
                            .unwrap_or_else(|_| "{\"error\":\"render failed\"}".to_owned());
                        push_log(&mut logs, line);
                        last_update = now_label();
                    }
                    RealtimeUpdate::Raw(_) | RealtimeUpdate::Event(_) => {}
                }
            }
            event = input.next() => {
                match event {
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                        if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                            || (key.code == KeyCode::Char('c')
                                && key.modifiers.contains(KeyModifiers::CONTROL))
                        {
                            break;
                        }
                        if key.code == KeyCode::Char(' ') {
                            paused = !paused;
                        }
                    }
                    Some(Err(error)) => return Err(error).context("terminal input failed"),
                    Some(Ok(_)) | None => {}
                }
            }
            _ = ticker.tick() => {}
        }
    }

    stream_task.abort();
    Ok(())
}

fn push_log(logs: &mut VecDeque<String>, line: String) {
    logs.push_back(line);
    if logs.len() > LOG_LIMIT {
        logs.pop_front();
    }
}

fn now_label() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        % 86_400;
    format!(
        "{:02}:{:02}:{:02} UTC",
        seconds / 3_600,
        (seconds % 3_600) / 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_is_bounded() {
        let mut logs = VecDeque::new();
        for index in 0..=LOG_LIMIT {
            push_log(&mut logs, index.to_string());
        }
        assert_eq!(logs.len(), LOG_LIMIT);
        assert_eq!(logs.front().map(String::as_str), Some("1"));
    }
}
