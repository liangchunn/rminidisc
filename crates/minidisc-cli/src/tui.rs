//! Interactive playback TUI (`rmd control`).
//!
//! A full-screen ratatui interface that lists the disc's tracks (organized by
//! group when the disc defines groups), shows live playback status, and maps
//! keys to the `netmd` transport commands.
//!
//! USB reads on these devices are slow and blocking, so the design keeps work
//! to a minimum: the track list is fetched once at startup (and on demand with
//! `R`), while only the lightweight status snapshot is polled on the refresh
//! timer.

use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use log::{Level, LevelFilter};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use rusb::{DeviceHandle, GlobalContext};

use crate::logbuf::{self, LevelControl, LogBuffer};
use netmd::{DeviceStatus, PlaybackState};

/// Selectable log levels for the modal, in display order.
const LEVEL_CHOICES: [LevelFilter; 6] = [
    LevelFilter::Off,
    LevelFilter::Error,
    LevelFilter::Warn,
    LevelFilter::Info,
    LevelFilter::Debug,
    LevelFilter::Trace,
];

/// How often the status snapshot is refreshed while idle (stopped/paused).
const STATUS_POLL_IDLE: Duration = Duration::from_millis(1500);
/// How often the status snapshot is refreshed while playing, so the progress
/// bar advances smoothly. (USB reads are slow, so don't go too low.)
const STATUS_POLL_ACTIVE: Duration = Duration::from_millis(500);
/// Keyboard poll timeout; also bounds the status refresh latency.
const EVENT_POLL_TIMEOUT: Duration = Duration::from_millis(250);

type Backend = CrosstermBackend<Stdout>;

struct TrackRow {
    title: String,
    length: [u32; 4],
    encoding: String,
}

/// A row in the disc list: either a group header or a track. Track rows carry
/// the disc track index, which indexes into [`App::tracks`].
#[derive(Debug, PartialEq, Eq)]
enum Row {
    Group(String),
    Track(u16),
}

/// Builds the display rows from the disc's group structure, interleaving group
/// headers with their tracks. Group headers are only emitted when the disc
/// defines named groups; otherwise (or when the group list is empty) a flat
/// list of all tracks is produced. Track indices `>= track_count` are skipped.
fn build_rows(groups: &[netmd::RawTrackGroup], track_count: u16) -> Vec<Row> {
    if groups.is_empty() {
        return (0..track_count).map(Row::Track).collect();
    }

    let has_named = groups.iter().any(|g| g.name.is_some());
    let mut rows = Vec::new();
    for group in groups {
        if has_named {
            let label = match &group.name {
                Some(name) if !name.is_empty() => name.clone(),
                Some(_) => "<untitled group>".to_string(),
                None => "(ungrouped)".to_string(),
            };
            rows.push(Row::Group(label));
        }
        for &track in &group.tracks {
            if track < track_count {
                rows.push(Row::Track(track));
            }
        }
    }
    rows
}

struct App {
    handle: DeviceHandle<GlobalContext>,
    device_name: String,
    disc_title: String,
    /// Per-track details, indexed by disc track number.
    tracks: Vec<TrackRow>,
    /// Display rows (group headers interleaved with tracks).
    rows: Vec<Row>,
    list_state: ListState,
    status: Option<DeviceStatus>,
    message: String,
    last_status_poll: Instant,
    /// In-memory log capture and runtime level control.
    logs: LogBuffer,
    level: LevelControl,
    /// Whether the right-hand log panel is visible.
    show_logs: bool,
    /// When `Some`, the log-level modal is open with the given selection index.
    level_modal: Option<ListState>,
}

/// Restores the terminal on drop, even if a panic unwinds through the loop.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

/// Entry point for `rmd control`.
pub fn run(device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    // Install the in-memory logger before any device I/O so its log lines are
    // captured. Honor RUST_LOG for the initial level, defaulting to Info.
    let initial_level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|s| s.parse::<LevelFilter>().ok())
        .unwrap_or(LevelFilter::Info);
    let (logs, level) = logbuf::install(initial_level);

    let handle = netmd::open_device_matching(device)?;
    let (vendor, product) = netmd::device_ids(&handle)?;
    let device_name = netmd::supported_device(vendor, product)
        .map(|d| d.name.to_string())
        .unwrap_or_else(|| format!("{vendor:04x}:{product:04x}"));

    let mut app = App {
        handle,
        device_name,
        disc_title: String::new(),
        tracks: Vec::new(),
        rows: Vec::new(),
        list_state: ListState::default(),
        status: None,
        message: "Loading disc…".to_string(),
        last_status_poll: Instant::now() - STATUS_POLL_IDLE,
        logs,
        level,
        show_logs: false,
        level_modal: None,
    };
    app.reload_disc();
    app.select_first_track();

    enable_raw_mode().context("entering raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("entering alternate screen")?;
    let _guard = TerminalGuard;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("creating terminal")?;

    let result = event_loop(&mut terminal, &mut app);

    // _guard restores the terminal on drop.
    drop(terminal);
    let _ = netmd::close_device(&app.handle);
    result
}

fn event_loop(terminal: &mut Terminal<Backend>, app: &mut App) -> anyhow::Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        if event::poll(EVENT_POLL_TIMEOUT)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

                // When the level modal is open it captures all keys.
                if app.level_modal.is_some() {
                    app.handle_modal_key(key.code);
                    continue;
                }

                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if ctrl => return Ok(()),
                    KeyCode::Char(' ') => app.toggle_play_pause(),
                    KeyCode::Char('s') => app.transport("stop", netmd::stop),
                    KeyCode::Char('n') => app.transport("next", netmd::next_track),
                    KeyCode::Char('p') => app.transport("prev", netmd::previous_track),
                    KeyCode::Char('f') => app.transport("fast-forward", netmd::fast_forward),
                    KeyCode::Char('b') => app.transport("rewind", netmd::rewind),
                    KeyCode::Char('e') => app.transport("eject", netmd::eject_disc),
                    KeyCode::Char('R') => app.reload_disc(),
                    KeyCode::Char('t') => app.show_logs = !app.show_logs,
                    KeyCode::Char('l') => app.open_level_modal(),
                    KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
                    KeyCode::Down | KeyCode::Char('j') => app.select_next(),
                    KeyCode::Enter => app.goto_selected(),
                    _ => {}
                }
                // After an action, refresh status promptly.
                app.poll_status(true);
            }
        }

        if app.last_status_poll.elapsed() >= app.current_poll_interval() {
            app.poll_status(false);
        }
    }
}

impl App {
    fn reload_disc(&mut self) {
        self.message = "Reading disc…".to_string();
        self.disc_title = netmd::get_disc_title(&self.handle, false).unwrap_or_default();

        let mut rows = Vec::new();
        match netmd::get_track_count(&self.handle) {
            Ok(count) => {
                for track in 0..count as u16 {
                    let title =
                        netmd::get_track_title(&self.handle, track, false).unwrap_or_default();
                    let length = netmd::get_track_length(&self.handle, track).unwrap_or([0; 4]);
                    let encoding = match netmd::get_track_encoding(&self.handle, track) {
                        Ok((enc, ch)) => format!("{enc:?}/{ch:?}"),
                        Err(_) => "?".to_string(),
                    };
                    rows.push(TrackRow {
                        title,
                        length,
                        encoding,
                    });
                }
                self.message = format!("Loaded {} tracks", rows.len());
            }
            Err(e) => {
                self.message = format!("Failed to read tracks: {e}");
            }
        }
        self.tracks = rows;
        self.rebuild_rows();
        self.clamp_selection();
    }

    /// Builds the display rows by reading the disc's group structure and
    /// interleaving group headers with their tracks. Group headers are only
    /// shown when the disc actually defines named groups.
    fn rebuild_rows(&mut self) {
        let track_count = self.tracks.len() as u16;
        let groups = netmd::get_track_group_list(&self.handle).unwrap_or_default();
        self.rows = build_rows(&groups, track_count);
    }

    /// Returns the row index of the first selectable track row, if any.
    fn first_track_row(&self) -> Option<usize> {
        first_track_row(&self.rows)
    }

    fn select_first_track(&mut self) {
        self.list_state.select(self.first_track_row());
    }

    /// Ensures the current selection points at a valid track row after a
    /// reload, falling back to the first track.
    fn clamp_selection(&mut self) {
        let valid = self
            .list_state
            .selected()
            .filter(|&i| matches!(self.rows.get(i), Some(Row::Track(_))));
        match valid {
            Some(i) => self.list_state.select(Some(i)),
            None => self.select_first_track(),
        }
    }

    /// Poll faster while the transport is actively moving so the progress bar
    /// stays current; back off when stopped/paused to spare the slow USB link.
    fn current_poll_interval(&self) -> Duration {
        match self.status.map(|s| s.state) {
            Some(PlaybackState::Playing | PlaybackState::FastForward | PlaybackState::Rewind) => {
                STATUS_POLL_ACTIVE
            }
            _ => STATUS_POLL_IDLE,
        }
    }

    fn poll_status(&mut self, force: bool) {
        if !force && self.last_status_poll.elapsed() < self.current_poll_interval() {
            return;
        }
        self.last_status_poll = Instant::now();
        match netmd::get_device_status(&self.handle) {
            Ok(s) => self.status = Some(s),
            Err(e) => self.message = format!("status error: {e}"),
        }
    }

    fn transport<F, E>(&mut self, label: &str, f: F)
    where
        F: Fn(&DeviceHandle<GlobalContext>) -> std::result::Result<(), E>,
        E: std::fmt::Display,
    {
        match f(&self.handle) {
            Ok(()) => self.message = label.to_string(),
            Err(e) => self.message = format!("{label} failed: {e}"),
        }
    }

    fn toggle_play_pause(&mut self) {
        let playing = matches!(self.status.map(|s| s.state), Some(PlaybackState::Playing));
        if playing {
            self.transport("pause", netmd::pause);
        } else {
            self.transport("play", netmd::play);
        }
    }

    fn select_prev(&mut self) {
        let Some(current) = self.list_state.selected() else {
            self.select_first_track();
            return;
        };
        if let Some(i) = prev_track_row(&self.rows, current) {
            self.list_state.select(Some(i));
        }
    }

    fn select_next(&mut self) {
        let Some(current) = self.list_state.selected() else {
            self.select_first_track();
            return;
        };
        if let Some(i) = next_track_row(&self.rows, current) {
            self.list_state.select(Some(i));
        }
    }

    fn goto_selected(&mut self) {
        let Some(track) = self
            .list_state
            .selected()
            .and_then(|i| self.rows.get(i))
            .and_then(|r| match r {
                Row::Track(idx) => Some(*idx),
                Row::Group(_) => None,
            })
        else {
            return;
        };
        match netmd::goto_track(&self.handle, track) {
            Ok(t) => self.message = format!("seeked to track #{}", t + 1),
            Err(e) => self.message = format!("goto failed: {e}"),
        }
    }

    /// Opens the log-level modal, preselecting the current level. Also reveals
    /// the log panel so the effect of changing the level is visible.
    fn open_level_modal(&mut self) {
        let current = self.level.get();
        let idx = LEVEL_CHOICES
            .iter()
            .position(|&l| l == current)
            .unwrap_or(0);
        let mut state = ListState::default();
        state.select(Some(idx));
        self.level_modal = Some(state);
        self.show_logs = true;
    }

    fn handle_modal_key(&mut self, code: KeyCode) {
        let Some(state) = self.level_modal.as_mut() else {
            return;
        };
        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.level_modal = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = state.selected().unwrap_or(0);
                state.select(Some(i.saturating_sub(1)));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = state.selected().unwrap_or(0);
                state.select(Some((i + 1).min(LEVEL_CHOICES.len() - 1)));
            }
            KeyCode::Enter => {
                let i = state.selected().unwrap_or(0);
                let chosen = LEVEL_CHOICES[i];
                self.level.set(chosen);
                self.message = format!("log level set to {chosen}");
                self.level_modal = None;
            }
            _ => {}
        }
    }
}

fn draw(f: &mut Frame, app: &App) {
    // Split the screen into a main column and an optional log panel on the right.
    let body = if app.show_logs {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(40), Constraint::Percentage(40)])
            .split(f.area());
        draw_logs(f, cols[1], app);
        cols[0]
    } else {
        f.area()
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(3),    // track list
            Constraint::Length(3), // now-playing + progress
            Constraint::Length(4), // status
            Constraint::Length(3), // help
        ])
        .split(body);

    draw_header(f, chunks[0], app);
    draw_tracks(f, chunks[1], app);
    draw_now_playing(f, chunks[2], app);
    draw_status(f, chunks[3], app);
    draw_help(f, chunks[4]);

    if app.level_modal.is_some() {
        draw_level_modal(f, app);
    }
}

fn level_color(level: Level) -> Color {
    match level {
        Level::Error => Color::Red,
        Level::Warn => Color::Yellow,
        Level::Info => Color::Green,
        Level::Debug => Color::Cyan,
        Level::Trace => Color::DarkGray,
    }
}

fn draw_logs(f: &mut Frame, area: Rect, app: &App) {
    // Render the most recent lines that fit in the panel.
    let visible = (area.height.saturating_sub(2)) as usize;
    let entries = app.logs.snapshot(visible.max(1));
    let items: Vec<ListItem> = entries
        .iter()
        .map(|e| {
            let short_target = e.target.rsplit("::").next().unwrap_or(&e.target);
            let line = Line::from(vec![
                Span::styled(
                    format!("{:<5} ", e.level),
                    Style::default()
                        .fg(level_color(e.level))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{short_target}: "),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(e.message.clone()),
            ]);
            ListItem::new(line)
        })
        .collect();

    let title = format!(" Logs [{}] ({}) ", app.level.get(), app.logs.len());
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(list, area);
}

fn draw_level_modal(f: &mut Frame, app: &App) {
    let Some(state) = app.level_modal.as_ref() else {
        return;
    };
    let area = centered_rect(30, LEVEL_CHOICES.len() as u16 + 2, f.area());
    f.render_widget(Clear, area);

    let current = app.level.get();
    let items: Vec<ListItem> = LEVEL_CHOICES
        .iter()
        .map(|&lvl| {
            let marker = if lvl == current { "• " } else { "  " };
            ListItem::new(Line::from(format!("{marker}{lvl}")))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Log Level ")
                .border_style(Style::default().fg(Color::Magenta)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Magenta)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let mut s = state.clone();
    f.render_stateful_widget(list, area, &mut s);
}

/// Returns a `width`×`height` rectangle centered within `area` (sizes in cells).
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// Computes `(elapsed_seconds, total_seconds, ratio 0.0..=1.0)` for the
/// currently playing track, if known. Uses the device's reported elapsed time
/// and the cached track length.
fn now_playing_progress(app: &App) -> Option<(u32, u32, f64)> {
    let s = app.status?;
    let track = s.track? as usize;
    let time = s.time?;
    let row = app.tracks.get(track)?;
    // Elapsed: minute is already absolute (hour*60 + minute) from PlaybackTime.
    let elapsed = time.minute * 60 + time.second;
    // Track length is [h, m, s, f]; ignore the frame remainder for the bar.
    let total = (row.length[0] * 60 + row.length[1]) * 60 + row.length[2];
    let ratio = if total > 0 {
        (elapsed as f64 / total as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    Some((elapsed, total, ratio))
}

fn fmt_mmss(total_seconds: u32) -> String {
    format!("{:02}:{:02}", total_seconds / 60, total_seconds % 60)
}

fn draw_now_playing(f: &mut Frame, area: Rect, app: &App) {
    let playing_track = app.status.and_then(|s| s.track);
    let title = playing_track
        .and_then(|t| app.tracks.get(t as usize))
        .map(|row| {
            if row.title.is_empty() {
                "<untitled>".to_string()
            } else {
                row.title.clone()
            }
        })
        .unwrap_or_else(|| "—".to_string());

    let (label, ratio) = match now_playing_progress(app) {
        Some((elapsed, total, ratio)) => (
            format!("{}  {} / {}", title, fmt_mmss(elapsed), fmt_mmss(total)),
            ratio,
        ),
        None => (title, 0.0),
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Now Playing "),
        )
        .gauge_style(Style::default().fg(Color::Magenta).bg(Color::Black))
        .ratio(ratio)
        .label(label);
    f.render_widget(gauge, area);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let title = if app.disc_title.is_empty() {
        "<untitled>".to_string()
    } else {
        app.disc_title.clone()
    };
    let line = Line::from(vec![
        Span::styled(
            format!(" {} ", app.device_name),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  Disc: "),
        Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
    ]);
    let p = Paragraph::new(line).block(Block::default().borders(Borders::ALL).title(" rmd "));
    f.render_widget(p, area);
}

fn draw_tracks(f: &mut Frame, area: Rect, app: &App) {
    let playing_track = app.status.and_then(|s| s.track);
    let items: Vec<ListItem> = app
        .rows
        .iter()
        .map(|row| match row {
            Row::Group(label) => ListItem::new(Line::from(vec![Span::styled(
                format!("▾ {label}"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )])),
            Row::Track(idx) => {
                let i = *idx as usize;
                let t = &app.tracks[i];
                let marker = if Some(*idx as u32) == playing_track {
                    "▶ "
                } else {
                    "  "
                };
                let title = if t.title.is_empty() {
                    "<untitled>"
                } else {
                    t.title.as_str()
                };
                ListItem::new(Line::from(vec![
                    Span::raw(marker),
                    Span::styled(
                        format!("{:>2}. ", i + 1),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw(format!("{title:<32} ")),
                    Span::styled(
                        format!("{:02}:{:02}:{:02}", t.length[0], t.length[1], t.length[2]),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::raw("  "),
                    Span::styled(t.encoding.clone(), Style::default().fg(Color::Green)),
                ]))
            }
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Tracks "))
        .highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    let mut state = app.list_state.clone();
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let mut lines = Vec::new();
    match &app.status {
        Some(s) => {
            let state = format!("{:?}", s.state);
            let track = s
                .track
                .map(|t| format!("#{}", t + 1))
                .unwrap_or_else(|| "-".to_string());
            let time = s
                .time
                .map(|t| format!("{:02}:{:02}+{:03}", t.minute, t.second, t.frame))
                .unwrap_or_else(|| "--:--".to_string());
            lines.push(Line::from(vec![
                Span::styled("State: ", Style::default().fg(Color::DarkGray)),
                Span::styled(state, Style::default().add_modifier(Modifier::BOLD)),
                Span::raw("   "),
                Span::styled("Track: ", Style::default().fg(Color::DarkGray)),
                Span::raw(track),
                Span::raw("   "),
                Span::styled("Time: ", Style::default().fg(Color::DarkGray)),
                Span::styled(time, Style::default().fg(Color::Yellow)),
                Span::raw("   "),
                Span::styled("Disc: ", Style::default().fg(Color::DarkGray)),
                Span::raw(if s.disc_present { "yes" } else { "no" }),
            ]));
        }
        None => lines.push(Line::from("status: (none)")),
    }
    lines.push(Line::from(vec![
        Span::styled("» ", Style::default().fg(Color::Magenta)),
        Span::raw(app.message.clone()),
    ]));

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Status "))
        .wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled("space", Style::default().fg(Color::Cyan)),
        Span::raw(" play/pause  "),
        Span::styled("s", Style::default().fg(Color::Cyan)),
        Span::raw(" stop  "),
        Span::styled("n/p", Style::default().fg(Color::Cyan)),
        Span::raw(" next/prev  "),
        Span::styled("f/b", Style::default().fg(Color::Cyan)),
        Span::raw(" ff/rew  "),
        Span::styled("↑/↓", Style::default().fg(Color::Cyan)),
        Span::raw(" select  "),
        Span::styled("Enter", Style::default().fg(Color::Cyan)),
        Span::raw(" goto  "),
        Span::styled("e", Style::default().fg(Color::Cyan)),
        Span::raw(" eject  "),
        Span::styled("R", Style::default().fg(Color::Cyan)),
        Span::raw(" reload  "),
        Span::styled("t", Style::default().fg(Color::Cyan)),
        Span::raw(" logs  "),
        Span::styled("l", Style::default().fg(Color::Cyan)),
        Span::raw(" level  "),
        Span::styled("q", Style::default().fg(Color::Cyan)),
        Span::raw(" quit"),
    ]);
    let p = Paragraph::new(line).block(Block::default().borders(Borders::ALL).title(" Keys "));
    f.render_widget(p, area);
}

/// Index of the first track row (skipping group headers), if any.
fn first_track_row(rows: &[Row]) -> Option<usize> {
    rows.iter().position(|r| matches!(r, Row::Track(_)))
}

/// Nearest track row after `current`, skipping group headers.
fn next_track_row(rows: &[Row], current: usize) -> Option<usize> {
    (current + 1..rows.len()).find(|&i| matches!(rows[i], Row::Track(_)))
}

/// Nearest track row before `current`, skipping group headers.
fn prev_track_row(rows: &[Row], current: usize) -> Option<usize> {
    (0..current)
        .rev()
        .find(|&i| matches!(rows[i], Row::Track(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use netmd::RawTrackGroup;

    fn group(name: Option<&str>, tracks: &[u16]) -> RawTrackGroup {
        RawTrackGroup {
            name: name.map(str::to_string),
            full_width_name: None,
            tracks: tracks.to_vec(),
        }
    }

    #[test]
    fn build_rows_flat_when_no_named_groups() {
        let groups = vec![group(None, &[0, 1, 2])];
        assert_eq!(
            build_rows(&groups, 3),
            vec![Row::Track(0), Row::Track(1), Row::Track(2)]
        );
    }

    #[test]
    fn build_rows_empty_group_list_falls_back_to_flat() {
        assert_eq!(build_rows(&[], 2), vec![Row::Track(0), Row::Track(1)]);
    }

    #[test]
    fn build_rows_interleaves_headers_with_named_groups() {
        let groups = vec![
            group(None, &[3]),
            group(Some("First"), &[0, 1, 2]),
            group(Some("Second"), &[4]),
        ];
        assert_eq!(
            build_rows(&groups, 5),
            vec![
                Row::Group("(ungrouped)".to_string()),
                Row::Track(3),
                Row::Group("First".to_string()),
                Row::Track(0),
                Row::Track(1),
                Row::Track(2),
                Row::Group("Second".to_string()),
                Row::Track(4),
            ]
        );
    }

    #[test]
    fn build_rows_labels_empty_group_name() {
        let groups = vec![group(Some(""), &[0])];
        assert_eq!(
            build_rows(&groups, 1),
            vec![Row::Group("<untitled group>".to_string()), Row::Track(0)]
        );
    }

    #[test]
    fn build_rows_skips_out_of_range_tracks() {
        let groups = vec![group(Some("G"), &[0, 5])];
        assert_eq!(
            build_rows(&groups, 1),
            vec![Row::Group("G".to_string()), Row::Track(0)]
        );
    }

    #[test]
    fn navigation_skips_group_headers() {
        let rows = vec![
            Row::Group("a".into()),
            Row::Track(1),
            Row::Track(2),
            Row::Group("b".into()),
            Row::Track(4),
        ];
        assert_eq!(first_track_row(&rows), Some(1));
        assert_eq!(next_track_row(&rows, 2), Some(4));
        assert_eq!(next_track_row(&rows, 4), None);
        assert_eq!(prev_track_row(&rows, 4), Some(2));
        assert_eq!(prev_track_row(&rows, 1), None);
    }
}
