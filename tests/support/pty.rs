use anyhow::{Result, ensure};
use nix::{
    fcntl::{FcntlArg, FdFlag, fcntl},
    poll::{PollFd, PollFlags, poll},
    pty::{Winsize, openpty},
    sys::{
        signal::{Signal, kill},
        termios::{Termios, tcgetattr},
    },
    unistd::{Pid, setsid},
};
use std::{
    env,
    fs::File,
    io::{Read, Write},
    num::NonZeroU16,
    os::{fd::AsFd, unix::process::CommandExt},
    path::Path,
    process::{Child, Command},
    time::{Duration, Instant},
};

pub const BIN: &str = env!("CARGO_BIN_EXE_vroom");
pub const TIMEOUT: Duration = Duration::from_secs(10);
pub const KITTY_UPLOAD: &str = "\x1b_Gf=100,a=T,";

pub fn kitty_delete(id: u32) -> String {
    format!("\x1b_Gq=2,a=d,d=I,i={id}\x1b\\")
}

pub fn editor(video: &Path, kitty: bool) -> Command {
    let mut command = Command::new(BIN);
    command
        .arg(video)
        .env(
            "TERM",
            if kitty {
                "xterm-kitty"
            } else {
                "xterm-256color"
            },
        )
        .env_remove("KITTY_WINDOW_ID")
        .env_remove("TERM_PROGRAM");
    command
}

pub struct Session {
    child: Child,
    master: File,
    slave: File,
    original_mode: Termios,
    pub output: Vec<u8>,
    screen: vt100::Parser,
}

impl Session {
    pub fn start(command: Command) -> Result<Self> {
        Self::spawn(command, false)
    }

    pub fn spawn(mut command: Command, redirected: bool) -> Result<Self> {
        let size = Winsize {
            ws_row: 41,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let pty = openpty(&size, None)?;
        let master = File::from(pty.master);
        let slave = File::from(pty.slave);
        for fd in [&master, &slave] {
            fcntl(fd, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC))?;
        }
        let original_mode = tcgetattr(&slave)?;
        command
            .stdin(if redirected {
                File::open("/dev/null")?
            } else {
                slave.try_clone()?
            })
            .stdout(slave.try_clone()?)
            .stderr(slave.try_clone()?);
        // Detach from the test runner's controlling terminal so Crossterm
        // queries the pseudoterminal even when cargo is run interactively.
        // SAFETY: setsid is async-signal-safe in the forked child.
        unsafe {
            command.pre_exec(|| setsid().map(|_| ()).map_err(std::io::Error::from));
        }
        let child = command.spawn()?;
        Ok(Self {
            child,
            master,
            slave,
            original_mode,
            output: Vec::new(),
            screen: vt100::Parser::new(nonzero(size.ws_row), nonzero(size.ws_col), 0),
        })
    }

    pub fn send(&mut self, keys: &str) -> Result<()> {
        self.master.write_all(keys.as_bytes())?;
        Ok(())
    }

    pub fn read_output(&mut self, wait_ms: u16) -> Result<bool> {
        let mut fds = [PollFd::new(self.master.as_fd(), PollFlags::POLLIN)];
        if poll(&mut fds, wait_ms)? == 0
            || !fds[0]
                .revents()
                .is_some_and(|flags| flags.contains(PollFlags::POLLIN))
        {
            return Ok(false);
        }
        let mut bytes = [0; 8192];
        let count = self.master.read(&mut bytes)?;
        self.output.extend_from_slice(&bytes[..count]);
        self.screen.process(&bytes[..count]);
        Ok(count != 0)
    }

    pub fn tick(&mut self, deadline: Instant, description: &str) -> Result<()> {
        if let Some(status) = self.child.try_wait()? {
            self.drain_output()?;
            anyhow::bail!(
                "editor exited ({status}) while waiting for {description}\n{}",
                self.tail()
            );
        }
        ensure!(
            Instant::now() < deadline,
            "timed out waiting for {description}\n{}",
            self.tail()
        );
        self.read_output(30)?;
        Ok(())
    }

    pub fn drain_output(&mut self) -> Result<()> {
        while self.read_output(0)? {}
        Ok(())
    }

    // Use decoded screen state for text, because Ratatui may only transmit the
    // changed characters. Use the raw transcript for protocol and startup logs.
    pub fn wait_for_screen(&mut self, text: &str) -> Result<()> {
        let deadline = Instant::now() + TIMEOUT;
        while !self.screen.screen().contents().contains(text) {
            self.tick(deadline, text)?;
        }
        Ok(())
    }

    pub fn wait_for_wire(&mut self, from: usize, bytes: &str) -> Result<usize> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(offset) = self.output[from..]
                .windows(bytes.len())
                .position(|w| w == bytes.as_bytes())
            {
                return Ok(from + offset + bytes.len());
            }
            self.tick(deadline, &format!("wire sequence {bytes:?}"))?;
        }
    }

    pub fn wait_for_file(&mut self, path: &Path) -> Result<()> {
        let deadline = Instant::now() + TIMEOUT;
        while !path.exists() {
            self.tick(deadline, &path.display().to_string())?;
        }
        Ok(())
    }

    fn tail(&self) -> String {
        let raw = String::from_utf8_lossy(&self.output[self.output.len().saturating_sub(1200)..]);
        format!(
            "terminal screen:\n{}\nwire tail: {raw:?}",
            self.screen.screen().contents()
        )
    }

    pub fn signal(&self, signal: Signal) -> Result<()> {
        kill(Pid::from_raw(self.child.id().try_into()?), signal)?;
        Ok(())
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        rustix::termios::tcsetwinsize(
            &self.slave,
            rustix::termios::Winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )?;
        self.screen.set_size(nonzero(rows), nonzero(cols));
        self.signal(Signal::SIGWINCH)
    }

    pub fn finish(&mut self) -> Result<()> {
        let deadline = Instant::now() + TIMEOUT;
        let status = loop {
            if let Some(status) = self.child.try_wait()? {
                break status;
            }
            ensure!(
                Instant::now() < deadline,
                "editor did not exit; terminal output:\n{}",
                self.tail()
            );
            self.read_output(30)?;
        };
        // Even after waitpid reports exit, teardown escapes may still be in
        // the PTY buffer. Drain it without waiting for EOF (we own the slave).
        self.drain_output()?;
        let output = String::from_utf8_lossy(&self.output);
        assert!(status.success(), "{status}; {}", self.tail());
        assert!(output.contains("\x1b[?1049h")); // entered the alternate screen
        assert!(output.contains("\x1b[?1049l")); // restored the main screen
        assert!(output.contains("\x1b[?25h")); // restored the cursor
        assert!(output.contains("\x1b[?2004l")); // restored bracketed paste mode
        assert_eq!(tcgetattr(&self.slave)?, self.original_mode);
        Ok(())
    }
}

fn nonzero(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value).expect("test terminal dimensions must be positive")
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
