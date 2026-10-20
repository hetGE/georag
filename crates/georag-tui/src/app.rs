//! The loop: terminal events from a reader thread, a 250 ms tick, `update`,
//! then `view`, coalescing whatever arrived since the last draw.

use std::io::Write;
use std::time::Duration;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::execute;
use tokio::sync::mpsc;

use crate::TuiError;
use crate::state::{Cmd, Msg, State};
use crate::theme::Theme;
use crate::update::update;
use crate::view::draw;

pub const TICK: Duration = Duration::from_millis(250);

pub fn run(state: State, theme: Theme) -> Result<(), TuiError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(TuiError::Terminal)?;

    let mut terminal = ratatui::try_init().map_err(TuiError::Terminal)?;
    let mouse = state.mouse;
    if mouse {
        let _ = execute!(std::io::stdout(), EnableMouseCapture);
    }

    let result = runtime.block_on(event_loop(&mut terminal, state, theme));

    if mouse {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
    }
    ratatui::restore();
    let _ = std::io::stdout().flush();
    result
}

/// Reads crossterm events on a plain thread so the async loop never blocks.
fn spawn_reader(tx: mpsc::Sender<Msg>) {
    std::thread::Builder::new()
        .name("georag-tui-input".into())
        .spawn(move || {
            loop {
                if tx.is_closed() {
                    return;
                }
                match crossterm::event::poll(Duration::from_millis(100)) {
                    Ok(true) => match crossterm::event::read() {
                        Ok(Event::Key(k)) => {
                            if tx.blocking_send(Msg::Key(k)).is_err() {
                                return;
                            }
                        }
                        Ok(Event::Mouse(m)) => {
                            if tx.blocking_send(Msg::Mouse(m)).is_err() {
                                return;
                            }
                        }
                        Ok(Event::Resize(w, h)) => {
                            if tx.blocking_send(Msg::Resize(w, h)).is_err() {
                                return;
                            }
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::error!("terminal input failed: {e}");
                            return;
                        }
                    },
                    Ok(false) => {}
                    Err(e) => {
                        tracing::error!("terminal poll failed: {e}");
                        return;
                    }
                }
            }
        })
        .expect("spawning the input thread");
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    mut state: State,
    theme: Theme,
) -> Result<(), TuiError> {
    let (tx, mut rx) = mpsc::channel::<Msg>(256);
    spawn_reader(tx);
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let size = terminal.size().map_err(TuiError::Terminal)?;
    state.width = size.width;
    state.height = size.height;

    loop {
        terminal
            .draw(|f| draw(f, &state, &theme))
            .map_err(TuiError::Loop)?;

        let first = tokio::select! {
            msg = rx.recv() => match msg {
                Some(m) => m,
                None => return Ok(()),
            },
            _ = tick.tick() => Msg::Tick,
        };
        let mut batch = vec![first];
        while let Ok(m) = rx.try_recv() {
            batch.push(m);
        }
        for msg in batch {
            let cmds = update(&mut state, msg);
            if cmds.contains(&Cmd::Quit) || state.should_quit {
                return Ok(());
            }
        }
    }
}
