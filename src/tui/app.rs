// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

//! Terminal lifecycle and event loop.

use std::io::{self, IsTerminal, Stdout};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use super::client::UiCommand;
use super::input;
use super::model::{App, Capabilities, UiEvent};
use super::render;

const FALLBACK_TICK: Duration = Duration::from_millis(250);

enum InputMessage {
    Event(Event),
    Error(String),
}

/// Runs the TUI with a supplied event stream.
///
/// `commands` is used by the IPC manager to upgrade the selected instance to
/// the detail subscription while leaving all other connections in summary
/// mode.  Passing `None` is useful for renderer demos and tests.
pub async fn run_with_receiver(
    events: mpsc::Receiver<UiEvent>,
    commands: Option<mpsc::UnboundedSender<UiCommand>>,
) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("tui requires an interactive terminal");
    }

    let mut terminal = TerminalSession::enter().context("failed to initialize terminal")?;
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let stop_input = Arc::new(AtomicBool::new(false));
    let input_thread = spawn_input_thread(Arc::clone(&stop_input), input_tx);
    let result = run_loop(
        &mut terminal.terminal,
        events,
        commands.as_ref(),
        &mut input_rx,
    )
    .await;
    stop_input.store(true, Ordering::Release);
    let _ = input_thread.join();
    if let Some(commands) = commands {
        let _ = commands.send(UiCommand::Shutdown);
    }
    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    mut events: mpsc::Receiver<UiEvent>,
    commands: Option<&mpsc::UnboundedSender<UiCommand>>,
    input_rx: &mut mpsc::UnboundedReceiver<InputMessage>,
) -> Result<()> {
    let mut app = App::default();
    app.capabilities = terminal_capabilities();
    let mut selected = None;
    let mut tick = tokio::time::interval(FALLBACK_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut events_open = true;
    let mut redraw = true;

    loop {
        if redraw {
            terminal
                .draw(|frame| render::render(frame, &app))
                .context("failed to draw terminal")?;
            redraw = false;
        }
        if app.should_quit {
            break;
        }

        tokio::select! {
            input = input_rx.recv() => {
                match input {
                    Some(InputMessage::Event(Event::Key(key))) => {
                        redraw |= input::handle_key(&mut app, key);
                    }
                    Some(InputMessage::Event(Event::Resize(_, _))) => redraw = true,
                    Some(InputMessage::Event(_)) => {}
                    Some(InputMessage::Error(message)) => {
                        app.global_error = Some(message);
                        redraw = true;
                    }
                    None => {}
                }
            }
            event = events.recv(), if events_open => {
                match event {
                    Some(event) => {
                        app.apply(event);
                        redraw = true;
                    }
                    None => {
                        events_open = false;
                        app.global_error = Some("telemetry client stopped".to_owned());
                        redraw = true;
                    }
                }
            }
            _ = tick.tick() => {
                app.tick(Instant::now());
                redraw = true;
            }
        }

        let next_selected = app.selected_id().map(str::to_owned);
        if next_selected != selected {
            selected.clone_from(&next_selected);
            if let Some(commands) = commands {
                let _ = commands.send(UiCommand::Select(next_selected));
            }
        }
    }
    Ok(())
}

fn spawn_input_thread(
    stop: Arc<AtomicBool>,
    sender: mpsc::UnboundedSender<InputMessage>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            match event::poll(FALLBACK_TICK) {
                Ok(true) => match event::read() {
                    Ok(event) => {
                        if sender.send(InputMessage::Event(event)).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(InputMessage::Error(error.to_string()));
                        return;
                    }
                },
                Ok(false) => {}
                Err(error) => {
                    let _ = sender.send(InputMessage::Error(error.to_string()));
                    return;
                }
            }
        }
    })
}

fn terminal_capabilities() -> Capabilities {
    let terminal = std::env::var("TERM").unwrap_or_default();
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok());
    let unicode = locale.is_none_or(|locale| {
        let normalized = locale.to_ascii_lowercase();
        normalized.contains("utf-8") || normalized.contains("utf8")
    });
    Capabilities {
        unicode,
        color: terminal != "dumb" && std::env::var_os("NO_COLOR").is_none(),
    }
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalSession {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, Hide) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => {
                let mut session = Self { terminal };
                session.terminal.clear()?;
                Ok(session)
            }
            Err(error) => {
                let _ = disable_raw_mode();
                let mut stdout = io::stdout();
                let _ = execute!(stdout, Show, LeaveAlternateScreen);
                Err(error)
            }
        }
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), Show, LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

#[cfg(test)]
#[path = "../tests/tui/app.rs"]
mod tests;
