use std::{
    io::{self, IsTerminal, stdout},
    time::Instant,
};

use anyhow::{Context, Result, ensure};
use crossterm::{
    cursor::{Hide, Show},
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen, SetTitle},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::{
    sync::mpsc,
    time::{Instant as TokioInstant, sleep_until},
};

use crate::{model::Project, paths::export_path, video};

mod app;
mod kitty;
mod view;

use app::{App, Effect};
use view::Layout;

pub async fn run(project: Project) -> Result<()> {
    ensure!(
        stdout().is_terminal(),
        "vroom needs an interactive terminal"
    );
    setup_input()?;
    let kitty = kitty::supported();
    #[cfg(unix)]
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    #[cfg(not(unix))]
    let mut interrupt = std::pin::pin!(tokio::signal::ctrl_c());
    let _screen = Screen::enter(kitty)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut events = terminal_events()?;

    let mut app = App::new(project);
    let source = app.project.source.clone();
    let output = export_path(&source);
    let mut layout = Layout::new(terminal.size()?, app.project.info);
    let mut image = kitty::Preview::default();
    let mut preview = None;
    let mut export = None;
    let mut dirty = true;
    let mut request_preview = true;

    loop {
        if dirty {
            let mut next_layout = layout;
            terminal.draw(|frame| next_layout = view::draw(frame, &app, kitty))?;
            if next_layout != layout {
                if kitty {
                    image.clear(terminal.backend_mut())?;
                }
                layout = next_layout;
                request_preview = true;
            }
            dirty = false;
        }
        if request_preview {
            // A seek/resize supersedes the previous request, even at the
            // same time. Dropping the future terminates its FFmpeg process.
            preview = Some(Box::pin(video::extract_frame(
                &source,
                app.cursor,
                layout.frame_pixel_height(),
            )));
            request_preview = false;
        }

        tokio::select! {
            result = async {
                #[cfg(unix)]
                {
                    tokio::select! { _ = interrupt.recv() => {}, _ = terminate.recv() => {} }
                    std::io::Result::Ok(())
                }
                #[cfg(not(unix))]
                { interrupt.as_mut().await }
            } => { result?; break; }
            event = events.recv() => match event.transpose()? {
                Some(Event::Key(key)) => {
                    match app.key(key, Instant::now()) {
                        Effect::Quit => break,
                        Effect::Preview => request_preview = true,
                        Effect::Export => {
                            let segments: Vec<_> = app.project.segments().collect();
                            let (source, output) = (&source, &output);
                            export = Some(Box::pin(async move {
                                video::export(source, &segments, output).await
                            }));
                        }
                        Effect::None => {}
                    }
                    dirty = true;
                }
                Some(Event::Resize(..)) => {
                    // Ratatui invalidates its buffers when the viewport size
                    // changes. Placement is redrawn after the new text layout.
                    request_preview = true;
                    dirty = true;
                }
                Some(_) => {}
                None => break,
            },
            result = async { preview.as_mut().unwrap().await }, if preview.is_some() => {
                preview = None;
                match result {
                    Ok(png) if !png.is_empty() => {
                        // A resize may have occurred before its input event
                        // reached us. Never place an image in the old viewport.
                        if kitty && Layout::new(terminal.size()?, app.project.info) != layout {
                            dirty = true;
                            request_preview = true;
                            continue;
                        }
                        if kitty { image.draw(terminal.backend_mut(), &png, layout)?; }
                        request_preview = app.frame_ready(Instant::now());
                        dirty = true;
                    }
                    _ => {
                        // Don't leave a previous frame under a new playhead.
                        if kitty {
                            image.clear(terminal.backend_mut())?;
                        }
                        app.frame_failed();
                    }
                }
            }
            result = async { export.as_mut().unwrap().await }, if export.is_some() => {
                export = None;
                app.export_finished(result, Instant::now());
                dirty = true;
            }
            _ = async {
                let deadline = app.status_deadline().unwrap();
                sleep_until(TokioInstant::from_std(deadline)).await;
            }, if app.status_deadline().is_some() => {
                app.clear_status();
                dirty = true;
            }
        }
    }
    Ok(())
}

fn setup_input() -> Result<()> {
    if io::stdin().is_terminal() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        // Bubble Tea reopens the controlling terminal if stdin is redirected.
        // Open its actual device, not the /dev/tty alias (which Crossterm's
        // kqueue input backend cannot poll on macOS).
        let path =
            rustix::termios::ttyname(stdout(), Vec::new()).context("locate terminal input")?;
        let input = rustix::fs::openat(
            rustix::fs::CWD,
            path.as_c_str(),
            rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOCTTY,
            rustix::fs::Mode::empty(),
        )
        .context("open terminal device")?;
        // Install the descriptor before Crossterm starts reading stdin.
        rustix::stdio::dup2_stdin(&input).context("set terminal input")?;
        Ok(())
    }
    #[cfg(not(unix))]
    anyhow::bail!("vroom needs an interactive terminal");
}

fn terminal_events() -> io::Result<mpsc::Receiver<io::Result<Event>>> {
    let (sender, receiver) = mpsc::channel(32);
    // Crossterm can block waiting for the rest of a split escape sequence.
    // Keep it off the UI executor so FFmpeg and signal handling still run.
    // Don't join on exit: a terminal read can remain blocked until process exit.
    std::thread::Builder::new()
        .name("vroom-input".into())
        .spawn(move || {
            while !sender.is_closed() {
                let event = event::read();
                let failed = event.is_err();
                if sender.blocking_send(event).is_err() || failed {
                    break;
                }
            }
        })?;
    Ok(receiver)
}

struct Screen {
    kitty: bool,
}

impl Screen {
    fn enter(kitty: bool) -> Result<Self> {
        terminal::enable_raw_mode().context("enable terminal raw mode")?;
        let screen = Self { kitty };
        execute!(
            stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            SetTitle("vroom"),
            Hide
        )?;
        Ok(screen)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let mut stdout = stdout();
        if self.kitty {
            let _ = kitty::cleanup(&mut stdout);
        }
        let _ = execute!(
            stdout,
            DisableBracketedPaste,
            Show,
            LeaveAlternateScreen,
            SetTitle("")
        );
        let _ = terminal::disable_raw_mode();
    }
}
