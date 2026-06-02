// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Brandon Huey <hello@aftertone.co>

//! `brumectl watch` — live three-panel dashboard for in-session dev work.
//!
//! Panels:
//!   - DEVICE: live connection state + vitals (reuses status::probe).
//!   - CONSOLE: streaming log events from the device. (Stub: no stream
//!     source on the device side yet.)
//!   - WATCHING: local ./scripts/ file watcher + push status. (Stub:
//!     no watcher wired yet; coming with brumectl push.)
//!
//! Key bindings:
//!   q / Esc / Ctrl-C → quit
//!   r                → force an out-of-band probe refresh
//!
//! A background thread probes every 2s via an mpsc channel so the UI
//! stays responsive while the ssh round-trip is in flight.

use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use super::status::{DeviceInfo, format_uptime, probe};
use super::target::Target;

const PROBE_INTERVAL: Duration = Duration::from_secs(2);
const FRAME_POLL: Duration = Duration::from_millis(100);

/// The result of a probe attempt — Ok on success, the error string on
/// failure so we can keep rendering the old info alongside a warning.
type ProbeResult = Result<DeviceInfo, String>;

#[derive(Debug)]
struct App {
    target: Target,
    last_info: Option<DeviceInfo>,
    last_error: Option<String>,
    last_update: Option<Instant>,
    connected: bool,
    probe_count: u32,
}

impl App {
    fn new(target: Target) -> Self {
        Self {
            target,
            last_info: None,
            last_error: None,
            last_update: None,
            connected: false,
            probe_count: 0,
        }
    }

    fn apply(&mut self, result: ProbeResult) {
        self.probe_count += 1;
        self.last_update = Some(Instant::now());
        match result {
            Ok(info) => {
                self.last_info = Some(info);
                self.last_error = None;
                self.connected = true;
            }
            Err(e) => {
                self.last_error = Some(e);
                self.connected = false;
            }
        }
    }

    fn last_update_label(&self) -> String {
        match self.last_update {
            None => "never".to_string(),
            Some(t) => {
                let secs = t.elapsed().as_secs();
                if secs < 3 {
                    "just now".to_string()
                } else {
                    format!("{secs}s ago")
                }
            }
        }
    }
}

pub fn run(target: &Target) -> Result<()> {
    let mut app = App::new(target.clone());

    // Probe thread — owns the ssh round-trip so the main thread never blocks.
    // refresh_tx wakes it early when the user hits 'r'.
    let (probe_tx, probe_rx) = mpsc::channel::<ProbeResult>();
    let (refresh_tx, refresh_rx) = mpsc::channel::<()>();
    let probe_target = target.clone();
    let _probe_handle = thread::spawn(move || probe_loop(probe_target, probe_tx, refresh_rx));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, &mut app, &probe_rx, &refresh_tx);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn probe_loop(target: Target, tx: mpsc::Sender<ProbeResult>, refresh: mpsc::Receiver<()>) {
    let send = |r: Result<super::status::ProbeOk, anyhow::Error>| {
        tx.send(r.map(|ok| ok.info).map_err(|e| e.to_string()))
            .is_ok()
    };
    if !send(probe(&target)) {
        return;
    }
    loop {
        match refresh.recv_timeout(PROBE_INTERVAL) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {
                if !send(probe(&target)) {
                    return;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    probe_rx: &mpsc::Receiver<ProbeResult>,
    refresh_tx: &mpsc::Sender<()>,
) -> Result<()> {
    loop {
        while let Ok(result) = probe_rx.try_recv() {
            app.apply(result);
        }

        terminal.draw(|f| render(f, app))?;

        if crossterm::event::poll(FRAME_POLL)? {
            if let Event::Key(KeyEvent {
                code,
                modifiers,
                kind,
                ..
            }) = crossterm::event::read()?
            {
                if kind != KeyEventKind::Press {
                    continue;
                }
                match (code, modifiers) {
                    (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => return Ok(()),
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(()),
                    (KeyCode::Char('r'), _) => {
                        let _ = refresh_tx.send(());
                    }
                    _ => {}
                }
            }
        }
    }
}

fn render(f: &mut Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // header
            Constraint::Length(11), // device panel
            Constraint::Min(6),     // console panel
            Constraint::Length(8),  // watching panel
            Constraint::Length(1),  // footer
        ])
        .split(f.area());

    render_header(f, outer[0], app);
    render_device(f, outer[1], app);
    render_console(f, outer[2], app);
    render_watching(f, outer[3], app);
    render_footer(f, outer[4], app);
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let status = if app.connected {
        Span::styled("● connected", Style::default().fg(Color::Green))
    } else if app.probe_count == 0 {
        Span::styled("○ probing…", Style::default().fg(Color::Yellow))
    } else {
        Span::styled("✕ unreachable", Style::default().fg(Color::Red))
    };
    let line = Line::from(vec![
        Span::styled("brumectl", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(&app.target.ssh_target, Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        status,
        Span::raw("  "),
        Span::styled(
            format!("(updated {})", app.last_update_label()),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    let p = Paragraph::new(line).block(Block::default().borders(Borders::BOTTOM));
    f.render_widget(p, area);
}

fn render_device(f: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line> = Vec::new();
    if let Some(info) = &app.last_info {
        lines.push(kv("firmware", &info.firmware));
        lines.push(kv("kernel", &info.kernel));
        lines.push(kv("uptime", &format_uptime(info.uptime)));
        lines.push(kv(
            "memory",
            &format!(
                "{} MB used / {} MB available",
                info.mem_used_mb, info.mem_avail_mb
            ),
        ));
        if let Some(c) = info.temp_celsius {
            lines.push(kv("temp", &format!("{c:.1} °C")));
        }
        lines.push(kv(
            "brume",
            &info.brume_pid.as_ref().map_or_else(
                || "not running".to_string(),
                |p| format!("running (pid {p})"),
            ),
        ));
        let mut net_parts = Vec::new();
        if let Some(a) = &info.eth_addr {
            net_parts.push(format!("eth0 {a}"));
        }
        if let Some(a) = &info.wlan_addr {
            net_parts.push(format!("wlan0 {a}"));
        }
        if !net_parts.is_empty() {
            lines.push(kv("network", &net_parts.join(", ")));
        }
    } else if let Some(err) = &app.last_error {
        lines.push(Line::from(Span::styled(
            err.clone(),
            Style::default().fg(Color::Red),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "probing…",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        " DEVICE ",
        Style::default().add_modifier(Modifier::BOLD),
    ));
    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn render_console(f: &mut Frame, area: Rect, _app: &App) {
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        " CONSOLE ",
        Style::default().add_modifier(Modifier::BOLD),
    ));
    let lines: Vec<Line> = vec![
        Line::from(Span::styled(
            "no log stream wired yet — Brume needs a /var/log/brume.jsonl (or",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "stdout to syslog) we can tail. Planned for brumectl logs.",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn render_watching(f: &mut Frame, area: Rect, _app: &App) {
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        " WATCHING ",
        Style::default().add_modifier(Modifier::BOLD),
    ));
    let lines: Vec<Line> = vec![
        Line::from(Span::styled(
            "file watcher + hot-reload push not wired yet — needs the device-side",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "scripting reload hook. Planned alongside brumectl push.",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn render_footer(f: &mut Frame, area: Rect, _app: &App) {
    let hint = Line::from(vec![
        Span::styled("  q", Style::default().fg(Color::Yellow)),
        Span::raw(" quit  "),
        Span::styled("r", Style::default().fg(Color::Yellow)),
        Span::raw(" refresh  "),
        Span::styled("esc", Style::default().fg(Color::Yellow)),
        Span::raw(" quit"),
    ]);
    f.render_widget(
        Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

fn kv(key: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{key:<10}"), Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::styled(value.to_string(), Style::default().fg(Color::White)),
    ])
}
