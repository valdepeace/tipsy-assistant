//! HUD drawing.
//!
//! A port of the proof of concept HUD (`poc/src/main.rs`) onto [`UiState`]: the clock, the system
//! figures and the audio envelopes all arrive through the state, so nothing here reads a clock or
//! probes the machine.

use std::f64::consts::TAU;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint::{Length, Min, Percentage};
use ratatui::layout::{Alignment, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Circle, Context, Line as CLine, Points};
use ratatui::widgets::{
    Axis, Block, BorderType, Chart, Dataset, Gauge, GraphType, LegendPosition, List, ListItem, ListState, Paragraph,
    Sparkline, Widget,
};
use tipsy_protocol::{AssistantState, ListeningMode, UiLanguage};

use crate::config_form::ConfigAction;
use crate::state::{KeyInput, LogEntry, Tab, UiState, Who};
use crate::strings::Strings;

const CYAN: Color = Color::Rgb(0, 210, 255);
const DIM: Color = Color::Rgb(0, 95, 125);
const GOLD: Color = Color::Rgb(255, 190, 60);
const SPINNER: [char; 4] = ['|', '/', '-', '\\'];
/// Milliseconds between two samples of a voice envelope, as `UiState` records them.
const ENVELOPE_HOP_MS: f64 = 50.0;
/// Envelope samples on screen in the oscilloscope: three seconds' worth.
const VOICE_WINDOW: f64 = 60.0;
/// Share of the width the clock and the log get. The proof of concept made it 28%; 30 is the chosen
/// compromise: narrow enough to leave the core in the middle of the HUD, and wide enough that a
/// reply wraps into a couple of lines instead of a column of single words.
const LEFT_PERCENT: u16 = 30;
/// World half-extent the rings are drawn in when the panel has room for it. The outermost ring has
/// radius 46, so this leaves it a margin instead of touching the border.
const RING_VIEW: f64 = 50.0;

pub struct ClockView {
    pub time: String,
    pub weekday: u8,
    pub day: u8,
    pub month: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub area: Rect,
    /// The keystroke this control answers, and the whole of what a click used to report. It stays
    /// because the HUD's titles are keys before they are anything else, and because a control whose
    /// click means exactly what its key means is honest about it.
    pub key: KeyInput,
    /// What the control *is*, for a control whose click is not the keystroke of whatever happens to
    /// be focused. The configuration screen's tabs, rows and buttons each name their own target
    /// here, and the client dispatches that action instead of a key — which is what keeps a click on
    /// "Profiles" from arriving as `F9`, the door key that closes the screen and drops the edits.
    /// `None` for the HUD's own titles, which are keys in the hand too.
    pub action: Option<ConfigAction>,
}

pub fn draw(frame: &mut Frame, state: &UiState, clock: &ClockView, title: &str) -> Vec<Hit> {
    let [header, body, input, help] = Layout::vertical([Length(1), Min(10), Length(3), Length(1)]).areas(frame.area());
    // The screen is full screen (spec §9): while it is up the HUD is not drawn at all, and its
    // clickable rows are not reported either — a click on the screen may not clear the conversation.
    // What *is* reported is the header's own two controls, and they are reported first, before the
    // screen's tabs and rows, while the screen is up exactly as when it is down. That order is the
    // frozen HUD specification (the screen is drawn under the header, never over it), and it is also
    // what keeps a click on the gear reaching the gear: `hits[1]` is the gear either way, and its key
    // still closes the screen.
    if let Some(form) = &state.config_form {
        let _ = clock;
        let screen = crate::render_config::draw_form(frame, form, state.strings(), state.elapsed_ms);
        // The header is drawn last so its own cells — the mode indicator and the gear — win whatever
        // the full-screen form painted under them, and it shows the *screen's* title while the screen
        // is up: the header's line repaints its whole row, so the profile's title there would be
        // erased by it and the screen's own title would never be visible on the row it owns.
        //
        // That row is also the one `draw_form` draws the screen's own unsaved marker on
        // (`render_config::draw_title`), so the marker travels with the header instead: it is the one
        // thing on screen that says the edits are unsaved, and the header would erase it. The title
        // stays the plain screen name — the header uppercases it into the badge — and the marker is
        // drawn beside the badge in the copy's own case.
        let mut hits =
            draw_header(frame, state, header, state.strings().cfg_title, form.dirty.then_some(state.strings().cfg_dirty));
        hits.extend(screen);
        return hits;
    }
    let [left, center, right] = Layout::horizontal([Percentage(LEFT_PERCENT), Min(30), Percentage(26)]).areas(body);
    let [clock_area, log] = Layout::vertical([Length(5), Min(3)]).areas(left);
    draw_clock(frame, state, clock, clock_area);
    let hits = draw_left(frame, state, log);
    let [rings, voice] = Layout::vertical([Min(8), Length(6)]).areas(center);
    draw_rings(frame, state, rings);
    draw_voice(frame, state, voice);
    draw_system(frame, state, right);
    draw_input(frame, state, input);
    if !help.is_empty() {
        frame.render_widget(Line::from(state.strings().help).fg(DIM), help);
    }
    // The header is drawn last because it is read last on the row and is the last thing on screen:
    // its own symbols win the cells they occupy however the panes and the screen have painted them.
    let mut ordered = draw_header(frame, state, header, title, None);
    ordered.extend(hits);
    ordered
}

/// Badge with the profile's title, the listening-mode indicator, whatever the assistant is doing,
/// and the ⚙ button — whose click is reported as a [`Hit`], like the HUD's other titles.
///
/// `dirty` is the configuration screen's unsaved marker, drawn beside the badge while the screen is
/// open and the form has edits (`draw` hands it in): the header repaints the very row the screen
/// draws that marker on, so the screen cannot keep it and the header has to carry it. It is `None`
/// for the HUD, which has nothing unsaved.
///
/// The header's own two controls come first in the order `draw` reports, both answering `F9`: the
/// mode indicator whose area is the word itself, then the gear at the right edge. A frame too small
/// to draw either reports no `Hit` for it rather than a rectangle nobody can see. That is true while
/// the configuration screen is up as well as when it is down: the screen is drawn beneath the header,
/// so the gear is on screen and clickable either way.
fn draw_header(frame: &mut Frame, state: &UiState, area: Rect, title: &str, dirty: Option<&str>) -> Vec<Hit> {
    let (line, mode, gear) = header(state, area, title, dirty);
    frame.render_widget(line, area);
    [mode, gear]
        .into_iter()
        .filter(|hit| !hit.area.is_empty() && area.intersection(hit.area) == hit.area)
        .collect()
}

/// One header row: the line to draw, and the two clickable areas on it.
///
/// The row is built to exactly `area`'s width on purpose. `ratatui` lays a line's spans out one
/// after another and does not report where they landed, so the only way for the mode indicator and
/// the gear to sit in the columns their [`Hit`]s name is for everything in front of them to be
/// exactly as wide as the room they were given.
///
/// The mode sits immediately before the gear, at the right edge, so the gear is the last thing read
/// on the row, and the status — the one span whose length the user's own data decides, a `⚠` notice
/// or a failure — is cut to whatever is left between the badge and the mode. The live indicator,
/// while a call lasts, is the status: it is drawn first of all and nothing else on the row may
/// stand in front of it (Plan 2C, Review Focus 3). The unsaved marker is measured before the status
/// and takes its room from it: while the screen is open that marker is the only thing saying the
/// edits are unsaved, and a status the user cannot act on may be cut instead.
fn header(state: &UiState, area: Rect, title: &str, dirty: Option<&str>) -> (Line<'static>, Hit, Hit) {
    let strings = state.strings();
    let spin = SPINNER[(state.elapsed_ms / 150) as usize % SPINNER.len()];
    let mode = match state.listening_mode {
        ListeningMode::Push => strings.mode_push,
        ListeningMode::Auto => strings.mode_auto,
        ListeningMode::HandsFree => strings.mode_hands_free,
    };
    let status: Span = if state.live {
        // A live call outranks everything else in the header: it bills for as long as it lasts, so
        // no spinner and no notice may hide it (Review Focus 3).
        Span::from(format!(" ⏺ {} ", strings.live)).fg(Color::Black).bg(Color::Red).bold()
    } else if state.mic_open {
        Span::from(strings.listening).fg(Color::Red).bold()
    } else if state.busy() {
        let what = match state.status {
            AssistantState::Transcribing => strings.transcribing,
            AssistantState::Speaking => strings.speaking,
            _ => strings.thinking,
        };
        Span::from(format!("{spin} {what} {}s", state.turn_ms / 1000)).fg(CYAN)
    } else if let Some((text, _)) = &state.notice {
        Span::from(format!("⚠ {text}")).fg(GOLD).bold()
    } else {
        Span::from(format!("● {}", strings.ready)).fg(Color::Green)
    };
    let badge = Span::from(format!(" ◉ {} ", title.to_uppercase())).fg(Color::Black).bg(CYAN).bold();
    // The screen's unsaved marker, in the gold the screen's own title row draws it in. It is one
    // span, cut below like the status, so a narrow frame loses its tail rather than the mode.
    let dirty = dirty.map(|text| Span::from(format!("{text} ")).fg(GOLD).bold());
    // The mode is a word (§9: `push`, `auto`, `hands free`) and the microphone is the colour: red
    // while it is open, whatever the mode.
    let mode_span = Span::from(format!(" {mode} ")).fg(if state.mic_open { Color::Red } else { DIM });
    let gear = Span::from(" ⚙ ").fg(GOLD).bold();
    let gap = Span::from("  ");
    // Every width below is the width the span is *drawn* in, which is not what `Span::width`
    // reports for every symbol: it reads the modern UAX #11 tables and the renderer advances by
    // older ones on `◉`, `⚙` and `·`. The badge carries `◉`, and the title is the profile's own, set
    // in the profile's own variety, so the two tables differ by the profile's name.
    let gear_w = drawn_width(&gear);
    let mode_w = drawn_width(&mode_span);
    // A row that cannot hold both controls puts them at its first column rather than letting the two
    // rectangles overlap or sit off the frame; the clipping below then drops whichever does not fit,
    // which is what a terminal too small to draw them reports.
    let (mode_w, gear_w) = if mode_w + gear_w <= area.width as usize { (mode_w, gear_w) } else { (0, 0) };
    let gear_x = area.right().saturating_sub(gear_w as u16).max(area.x);
    let mode_x = gear_x.saturating_sub(mode_w as u16).max(area.x);
    let max_x = area.right();
    let mode_hit = Hit {
        area: Rect::new(mode_x, area.y, gear_x.saturating_sub(mode_x).min(max_x.saturating_sub(mode_x)), 1),
        key: KeyInput::F(9),
        action: Some(ConfigAction::Toggle),
    };
    let gear_hit = Hit {
        area: Rect::new(gear_x, area.y, gear_w.min(max_x.saturating_sub(gear_x) as usize) as u16, 1),
        key: KeyInput::F(9),
        action: Some(ConfigAction::Toggle),
    };
    // The status keeps whatever is left between the badge and the mode, and never more. The unsaved
    // marker is measured first: it is the one span on this row the user has to see, so the status —
    // a ready word, or a notice the screen itself repeats — takes what is left of the room.
    let used = drawn_width(&badge) + drawn_width(&gap);
    let room = usize::from(mode_x.saturating_sub(area.x));
    let dirty = dirty
        .map(|span| cut_to(span.content.as_ref(), span.style, room.saturating_sub(used)))
        .unwrap_or_default();
    let dirty_w = drawn_width(&dirty);
    let status = cut_to(status.content.as_ref(), status.style, room.saturating_sub(used + dirty_w));
    let padding = room.saturating_sub(used + dirty_w + drawn_width(&status));
    let line = Line::from(vec![badge, gap, dirty, status, Span::raw(" ".repeat(padding)), mode_span, gear]);
    (line, mode_hit, gear_hit)
}

/// The column `drawn_width` reads its answer from. A private-use character: no string table, no
/// profile and no event can put it on the header row, so a cell holding it is the marker.
const MARKER: &str = "\u{E000}";

/// The columns a span occupies when it is drawn, which is what every position in the header is
/// measured with: the width of the cells it writes.
///
/// A span is laid out by one width table and drawn by another, and the two disagree on `◉`, `⚙` and
/// `·` — each one column to [`Span::width`] and two as drawn. The span is therefore measured by
/// drawing it and reading the column the span *after* it lands in: its own trailing space is part of
/// its width and no symbol marks the end of it, so a marker is the only honest ruler.
///
/// The scratch row is wide enough for `Span::width` plus room for the two tables to disagree. A span
/// past that stops at the row's edge instead of growing it without bound, and the header is a
/// handful of spans on one row.
fn drawn_width(span: &Span) -> usize {
    let room = span.width().saturating_add(4);
    if room == 0 {
        return 0;
    }
    let area = Rect::new(0, 0, u16::try_from(room).unwrap_or(u16::MAX), 1);
    let mut scratch = Buffer::empty(area);
    Line::from(vec![span.clone(), Span::from(MARKER)]).render(area, &mut scratch);
    for x in 0..area.width {
        if scratch[(x, 0)].symbol() == MARKER {
            return x as usize;
        }
    }
    // The span filled the whole scratch row: it is at least as wide as the row it was given.
    room
}

/// `text` cut to `width` columns in the width the row is drawn in, keeping `style`. Cutting by
/// [`drawn_width`] is what keeps a cut status exactly as wide as the room its own rectangle promised
/// it.
fn cut_to(text: &str, style: Style, width: usize) -> Span<'static> {
    let mut cut = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = drawn_width(&Span::from(c.to_string()));
        if used + w > width {
            break;
        }
        used += w;
        cut.push(c);
    }
    Span::styled(cut, style)
}

fn draw_clock(frame: &mut Frame, state: &UiState, clock: &ClockView, area: Rect) {
    let strings = state.strings();
    let weekday = strings.weekdays.get(clock.weekday as usize).copied().unwrap_or("");
    let month = strings.months.get(clock.month as usize).copied().unwrap_or("");
    let date = match state.lang {
        UiLanguage::Es => format!("{weekday}, {} de {month}", clock.day),
        UiLanguage::En => format!("{weekday}, {month} {}", clock.day),
    };
    let text = vec![
        Line::from(clock.time.clone()).fg(Color::White).bold(),
        Line::from(""),
        Line::from(date).fg(CYAN),
    ];
    frame.render_widget(Paragraph::new(text).alignment(Alignment::Center).block(panel(strings.local_time)), area);
}

/// Left panel: chat or network tab, plus the clear button whose clicks are reported as [`Hit`]s.
fn draw_left(frame: &mut Frame, state: &UiState, area: Rect) -> Vec<Hit> {
    let strings = state.strings();
    let tab = |name: &str, on: bool| -> Span<'static> {
        if on {
            Span::from(format!(" {name} ")).fg(Color::Black).bg(CYAN).bold()
        } else {
            Span::from(format!(" {name} ")).fg(DIM)
        }
    };
    let title = Line::from(vec![
        Span::from(" "),
        tab(strings.chat, state.tab == Tab::Chat),
        Span::from("│").fg(DIM),
        tab(strings.network, state.tab == Tab::Network),
        Span::from(" ←→ ").fg(DIM),
    ]);
    let clear = format!(" {} ", strings.clear);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(DIM))
        .title(title)
        .title(Line::from(Span::from(clear.clone()).fg(Color::Black).bg(GOLD).bold()).right_aligned());

    // Ratatui does not report where it laid a title out, so the clickable rows are measured by
    // hand: the title starts after the top-left corner, and the right-aligned one ends before the
    // top-right corner.
    let row = |x: u16, w: u16| Rect::new(x, area.y, w.min(area.right().saturating_sub(x)), 1);
    let chat_w = strings.chat.chars().count() as u16 + 2;
    let net_w = strings.network.chars().count() as u16 + 2;
    let clear_w = clear.chars().count() as u16;
    let hits = vec![
        Hit { area: row(area.x + 2, chat_w), key: KeyInput::F(7), action: None },
        Hit { area: row(area.x + 2 + chat_w + 1, net_w), key: KeyInput::F(8), action: None },
        Hit { area: row(area.right().saturating_sub(1 + clear_w), clear_w), key: KeyInput::F(4), action: None },
    ];

    let inner = block.inner(area);
    frame.render_widget(block, area);
    if state.tab == Tab::Chat {
        draw_log(frame, state, inner);
    } else {
        draw_net(frame, state, inner);
    }
    hits
}

fn draw_log(frame: &mut Frame, state: &UiState, area: Rect) {
    if area.is_empty() {
        return;
    }
    // `List` does not wrap an item that is wider than the panel, so every entry is wrapped here.
    let items: Vec<ListItem> = state.log.iter().map(|entry| log_item(entry, state.strings(), area.width)).collect();
    let list = List::new(items).highlight_style(Style::new().fg(Color::Black).bg(CYAN).add_modifier(Modifier::BOLD));
    let mut list_state = ListState::default().with_selected(state.selected);
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// One run of entry text and the style it keeps wherever the wrap puts it.
struct Piece {
    text: String,
    style: Style,
}

/// The entry as the list draws it, wrapped to the width of the panel.
fn log_item(entry: &LogEntry, strings: &Strings, width: u16) -> ListItem<'static> {
    ListItem::new(wrap(log_pieces(entry, strings), width as usize))
}

/// The runs an entry is made of: the marker of an assistant entry or the prefix of a user one
/// first, then the text, and the timing on a reply that has been said.
fn log_pieces(entry: &LogEntry, strings: &Strings) -> Vec<Piece> {
    let piece = |text: &str, style: Style| Piece { text: text.to_string(), style };
    if entry.who == Who::User {
        vec![
            piece(&format!("{} ", strings.you), Style::new().fg(DIM)),
            piece(&entry.text, Style::new().fg(Color::White)),
        ]
    } else if let Some(error) = &entry.error {
        vec![piece("◆  ✗ ", Style::new().fg(Color::Red)), piece(error, Style::new().fg(Color::Red))]
    } else if let Some(first_audio_ms) = entry.first_audio_ms {
        vec![
            piece("◆  ", Style::new().fg(GOLD)),
            piece(&entry.text, Style::new().fg(CYAN)),
            piece(&format!(" {:.1}s", first_audio_ms as f64 / 1000.0), Style::new().fg(DIM)),
        ]
    } else {
        vec![piece("◆  ", Style::new().fg(GOLD)), piece(&entry.text, Style::new().fg(GOLD).italic())]
    }
}

/// Word-wraps styled runs into rows of at most `width` columns.
///
/// A row breaks on a space the text already has, and the space a break happens at is dropped; only a
/// single word that does not fit a row of its own is cut, and never inside a character. No text is
/// dropped, so the timing of a reply stays on screen wherever the wrap puts it.
fn wrap(pieces: Vec<Piece>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut row: Vec<Piece> = Vec::new();
    let mut used = 0;
    for mut word in pieces.into_iter().flat_map(split_words) {
        loop {
            if row.is_empty() {
                // The space a row broke at, or a word left empty by it, is not drawn.
                word.text = word.text.trim_start().to_string();
                if word.text.is_empty() {
                    break;
                }
            }
            let w = text_width(&word.text);
            if used + w <= width {
                used += w;
                row.push(word);
                break;
            }
            if row.is_empty() {
                // A word too long for a row of its own: draw as much of it as fits.
                let (head, tail) = cut_at(&word.text, width);
                let tail = tail.to_string();
                row.push(Piece { text: head.to_string(), style: word.style });
                lines.push(take_row(&mut row));
                used = 0;
                word.text = tail;
                if word.text.is_empty() {
                    break;
                }
                continue;
            }
            lines.push(take_row(&mut row));
            used = 0;
        }
    }
    if !row.is_empty() {
        lines.push(take_row(&mut row));
    }
    if lines.is_empty() {
        // An entry without any text still takes its row in the list.
        lines.push(Line::default());
    }
    lines
}

/// Splits a run into the words a row may break between, each keeping the space that follows it so
/// that unwrapped text is reproduced exactly as it came in.
fn split_words(piece: Piece) -> Vec<Piece> {
    piece.text.split_inclusive(' ').map(|text| Piece { text: text.to_string(), style: piece.style }).collect()
}

/// Turns the pieces a row has collected into a line, trimming the space a break left on it.
fn take_row(row: &mut Vec<Piece>) -> Line<'static> {
    if let Some(last) = row.last_mut() {
        last.text = last.text.trim_end().to_string();
    }
    let spans: Vec<Span> = row.drain(..).filter(|piece| !piece.text.is_empty()).map(|piece| Span::styled(piece.text, piece.style)).collect();
    Line::from(spans)
}

/// The columns `text` takes on screen, by ratatui's own reckoning.
fn text_width(text: &str) -> usize {
    Span::from(text).width()
}

/// Splits `text` after the characters that fill `width` columns, always keeping at least one
/// character: a symbol wider than the row still moves the cut forward, so a caller cannot loop for
/// ever on a row too narrow for it.
fn cut_at(text: &str, width: usize) -> (&str, &str) {
    let mut end = 0;
    let mut used = 0;
    for (i, c) in text.char_indices() {
        let w = text_width(c.encode_utf8(&mut [0; 4]));
        if used > 0 && used + w > width {
            break;
        }
        used += w;
        end = i + c.len_utf8();
    }
    (&text[..end], &text[end..])
}

fn draw_net(frame: &mut Frame, state: &UiState, area: Rect) {
    let history = &state.net_history;
    // A panel this small cannot hold axes; it stays empty rather than jumbled.
    if area.width < 8 || area.height < 3 || history.len() < 2 {
        return;
    }
    let inbound: Vec<(f64, f64)> = history.iter().enumerate().map(|(i, h)| (i as f64, h.0)).collect();
    let outbound: Vec<(f64, f64)> = history.iter().enumerate().map(|(i, h)| (i as f64, h.1)).collect();
    let top = history.iter().map(|h| h.0.max(h.1)).fold(1024.0, f64::max) * 1.2;
    let datasets = vec![
        Dataset::default()
            .name(format!("IN  {}", human(state.net_in)))
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().fg(CYAN))
            .data(&inbound),
        Dataset::default()
            .name(format!("OUT {}", human(state.net_out)))
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::new().fg(GOLD))
            .data(&outbound),
    ];
    let chart = Chart::new(datasets)
        // The default constraints hide the legend in a panel as narrow as this one; these show it
        // as soon as it fits and keep it away when it does not.
        .hidden_legend_constraints((Length(18), Length(6)))
        .x_axis(
            Axis::default()
                .bounds([0.0, (history.len() - 1) as f64])
                .style(Style::new().fg(DIM))
                .labels(["-2 min".fg(DIM), "ahora".fg(DIM)]),
        )
        .y_axis(
            Axis::default()
                .bounds([0.0, top])
                .style(Style::new().fg(DIM))
                .labels(["0".fg(DIM), human(top).fg(DIM)]),
        )
        .legend_position(Some(LegendPosition::TopLeft));
    frame.render_widget(chart, area);
}

/// The world window the rings are scaled into: at least [`RING_VIEW`] on each axis, and widened
/// until the whole drawing fits the panel.
///
/// Terminal cells are twice as tall as wide, so half as much world per cell is spanned across as
/// down — the same world distance per cell on both axes is exactly what keeps the circles round.
/// The axis that is tight for the panel ends up at `RING_VIEW` and is filled by the drawing; the
/// other one only grows, so nothing is ever clipped.
fn ring_bounds(inner: Rect) -> (f64, f64) {
    let w = f64::from(inner.width.max(1));
    let h = f64::from(inner.height.max(1));
    let yr = RING_VIEW.max(2.0 * RING_VIEW * h / w);
    (yr * w / (2.0 * h), yr)
}

fn draw_rings(frame: &mut Frame, state: &UiState, area: Rect) {
    if area.is_empty() {
        return;
    }
    let t = state.elapsed_ms as f64 / 1000.0;
    let level = state.voice_level() as f64;
    let speed = if state.busy() { 2.2 } else { 0.5 };
    let block = panel(state.strings().core);
    let (xr, yr) = ring_bounds(block.inner(area));
    let canvas = Canvas::default()
        .block(block)
        .marker(Marker::Braille)
        .x_bounds([-xr, xr])
        .y_bounds([-yr, yr])
        .paint(move |ctx| {
            ctx.draw(&Circle { x: 0.0, y: 0.0, radius: 46.0, color: DIM });
            for (r, n, dir, color) in [(40.0, 3, 1.0, CYAN), (33.0, 6, -1.6, DIM), (27.0, 2, 2.4, GOLD)] {
                for k in 0..n {
                    let a0 = t * speed * dir + k as f64 * TAU / n as f64;
                    arc(ctx, r, a0, a0 + TAU / (n as f64 * 2.0), color);
                }
            }
            let pulse = 10.0 + 12.0 * level;
            ctx.draw(&Circle {
                x: 0.0,
                y: 0.0,
                radius: pulse,
                color: if level > 0.05 { Color::White } else { CYAN },
            });
            ctx.print(-2.0, 2.0, Span::from("◉").fg(Color::White).bold());
        });
    frame.render_widget(canvas, area);
}

/// Oscilloscope: a calm line that shakes with the loudness. Cyan for the assistant's envelope,
/// red for the microphone, in live.
fn draw_voice(frame: &mut Frame, state: &UiState, area: Rect) {
    if area.is_empty() {
        return;
    }
    let (levels, head, color): (&[f32], f64, Color) = if state.mic_open {
        (&state.mic_levels, state.mic_levels.len() as f64, Color::Red)
    } else {
        (&state.voice_envelope, state.voice_pos_ms as f64 / ENVELOPE_HOP_MS, CYAN)
    };
    let amp = |x: f64| {
        let j = head - VOICE_WINDOW + x * VOICE_WINDOW;
        if j < 0.0 { 0.0 } else { levels.get(j as usize).copied().unwrap_or(0.0) as f64 }
    };
    let t = state.elapsed_ms as f64 / 1000.0;
    let n = (area.width.saturating_sub(2) as usize * 2).max(2);
    let pts: Vec<(f64, f64)> = (0..=n)
        .map(|k| {
            let x = k as f64 / n as f64;
            // Two sines that do not agree give it a voice-like air; dragging them by t makes it flow.
            let wave = (x * 70.0 - t * 14.0).sin() * 0.7 + (x * 23.0 + t * 5.0).sin() * 0.3;
            (x, amp(x) * wave)
        })
        .collect();
    let canvas = Canvas::default()
        .block(panel(state.strings().voice))
        .marker(Marker::Braille)
        .x_bounds([0.0, 1.0])
        .y_bounds([-1.05, 1.05])
        .paint(move |ctx| {
            ctx.draw(&CLine { x1: 0.0, y1: 0.0, x2: 1.0, y2: 0.0, color: DIM });
            for w in pts.windows(2) {
                ctx.draw(&CLine { x1: w[0].0, y1: w[0].1, x2: w[1].0, y2: w[1].1, color });
            }
        });
    frame.render_widget(canvas, area);
}

fn draw_system(frame: &mut Frame, state: &UiState, area: Rect) {
    if area.is_empty() {
        return;
    }
    let block = panel(state.strings().system);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.is_empty() {
        return;
    }
    let [cpu, ram, _, net_in, net_out, _, history] = Layout::vertical([
        Length(1),
        Length(1),
        Length(1),
        Length(1),
        Length(1),
        Length(1),
        Min(1),
    ])
    .areas(inner);

    frame.render_widget(gauge("CPU", state.cpu.clamp(0.0, 100.0) as u64), cpu);
    frame.render_widget(gauge("RAM", state.ram.clamp(0.0, 100.0) as u64), ram);
    frame.render_widget(
        Line::from(vec![Span::from("IN  ").fg(DIM), Span::from(human(state.net_in)).fg(Color::White)]),
        net_in,
    );
    frame.render_widget(
        Line::from(vec![Span::from("OUT ").fg(DIM), Span::from(human(state.net_out)).fg(Color::White)]),
        net_out,
    );
    if !history.is_empty() {
        let data: Vec<u64> = state.cpu_history.iter().map(|v| v.clamp(0.0, 100.0) as u64).collect();
        frame.render_widget(Sparkline::default().data(&data).max(100).style(Style::new().fg(DIM)), history);
    }
}

fn draw_input(frame: &mut Frame, state: &UiState, area: Rect) {
    let strings = state.strings();
    let text = if state.input.is_empty() {
        Line::from(vec![
            Span::from("❯ ").fg(CYAN),
            Span::from(strings.placeholder).fg(DIM).italic(),
        ])
    } else {
        Line::from(vec![
            Span::from("❯ ").fg(CYAN),
            Span::from(state.input.clone()).fg(Color::White),
        ])
    };
    frame.render_widget(Paragraph::new(text).block(panel(strings.ask)), area);
    if area.width >= 2 && area.height >= 2 {
        // ponytail: the cursor counts characters, not width; emoji shift it a little
        let x = (area.x + 3 + state.input.chars().count() as u16).min(area.right() - 1);
        frame.set_cursor_position((x, area.y + 1));
    }
}

fn panel(title: &str) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(DIM))
        .title(Line::from(vec![
            Span::from(format!(" {title} ")).fg(CYAN).bold(),
            Span::from("● ").fg(Color::Green),
        ]))
}

fn gauge(label: &str, pct: u64) -> Gauge<'static> {
    Gauge::default()
        .gauge_style(Style::new().fg(CYAN).bg(Color::Rgb(0, 30, 40)))
        .percent(pct.min(100) as u16)
        .label(format!("{label} {pct}%"))
}

fn arc(ctx: &mut Context, r: f64, a0: f64, a1: f64, color: Color) {
    let coords: Vec<(f64, f64)> = (0..=60)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f64 / 60.0;
            (r * a.cos(), r * a.sin())
        })
        .collect();
    ctx.draw(&Points { coords: &coords, color });
}

fn human(bps: f64) -> String {
    match bps {
        b if b >= 1e6 => format!("{:.1} MB/s", b / 1e6),
        b if b >= 1e3 => format!("{:.1} KB/s", b / 1e3),
        b => format!("{b:.0} B/s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn es() -> &'static crate::strings::Strings {
        crate::strings::strings(UiLanguage::Es)
    }

    fn done(text: &str, first_audio_ms: u64) -> LogEntry {
        LogEntry {
            who: Who::Assistant,
            text: text.into(),
            first_audio_ms: Some(first_audio_ms),
            error: None,
            pending: false,
        }
    }

    fn rendered(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect::<String>())
            .collect()
    }

    #[test]
    fn wrap_breaks_only_on_spaces() {
        let pieces = vec![Piece { text: "uno dos tres cuatro".into(), style: Style::new() }];
        assert_eq!(rendered(&wrap(pieces, 8)), ["uno dos", "tres", "cuatro"]);
    }

    #[test]
    fn wrap_cuts_a_word_that_does_not_fit_a_row_and_never_inside_a_character() {
        let pieces = vec![Piece { text: "tú supercalifragilístico".into(), style: Style::new() }];
        assert_eq!(rendered(&wrap(pieces, 6)), ["tú", "superc", "alifra", "gilíst", "ico"]);
    }

    #[test]
    fn wrap_keeps_the_marker_first_and_the_timing_whole() {
        let rows = rendered(&wrap(log_pieces(&done("Ozú, quillo, eso es de backend", 1500), es()), 20));
        assert_eq!(rows.first().map(String::as_str), Some("◆  Ozú, quillo, eso"), "the marker opens the entry");
        assert!(rows.iter().any(|row| row.contains("1.5s")), "the timing must stay whole: {rows:?}");
        assert_eq!(rows.concat().replace(' ', ""), "◆Ozú,quillo,esoesdebackend1.5s", "nothing may be dropped");
    }

    #[test]
    fn wrap_keeps_a_user_prefix_on_the_first_row_and_wraps_an_error() {
        let mut entry = LogEntry {
            who: Who::User,
            text: "¿y el ticket de la cena?".into(),
            first_audio_ms: None,
            error: None,
            pending: false,
        };
        let rows = rendered(&wrap(log_pieces(&entry, es()), 12));
        assert_eq!(rows.first().map(String::as_str), Some("tú ¿y el"), "the prefix opens the entry");
        assert!(rows.iter().all(|row| text_width(row) <= 12), "{rows:?}");
        assert_eq!(rows.concat().replace(' ', ""), "tú¿yelticketdelacena?", "nothing may be dropped");

        entry.who = Who::Assistant;
        entry.error = Some("no hay conexión con el servidor".into());
        let rows = rendered(&wrap(log_pieces(&entry, es()), 12));
        assert!(rows.first().is_some_and(|row| row.starts_with("◆  ✗ ")), "{rows:?}");
        assert!(rows.iter().all(|row| text_width(row) <= 12), "{rows:?}");
        assert_eq!(rows.concat().replace(' ', ""), "◆✗nohayconexiónconelservidor");
    }

    #[test]
    fn wrap_wraps_a_pending_reply_too() {
        let mut entry = done("estoy pensando en la respuesta", 1500);
        entry.first_audio_ms = None;
        entry.pending = true;
        let rows = rendered(&wrap(log_pieces(&entry, es()), 12));
        assert_eq!(rows, ["◆  estoy", "pensando en", "la respuesta"]);
    }

    #[test]
    fn a_row_too_narrow_for_the_marker_still_terminates() {
        let rows = rendered(&wrap(log_pieces(&done("hola", 1500), es()), 1));
        assert_eq!(rows.concat(), "◆hola1.5s", "nothing may loop for ever or be lost: {rows:?}");
    }

    /// The scoped re-review's finding 1: the full-screen form writes `cfg_dirty` on the row the
    /// header repaints last, so the header carries the marker while the screen is open. It is the one
    /// thing on screen that says the edits are unsaved, and it is read off the header row itself —
    /// the row the screen's own title was drawn on and lost.
    #[test]
    fn the_unsaved_marker_survives_the_header_while_the_screen_is_open() {
        fn header_row(term: &Terminal<TestBackend>) -> String {
            let buffer = term.backend().buffer();
            (0..buffer.area.width).map(|x| buffer[(x, 0)].symbol().to_string()).collect()
        }

        let mut s = UiState::new();
        s.on_key(KeyInput::F(9));
        s.config_form.as_mut().unwrap().dirty = true;
        let clock = ClockView { time: "10:42:07".into(), weekday: 4, day: 10, month: 9 };
        let mut term = Terminal::new(TestBackend::new(150, 40)).unwrap();
        term.draw(|f| {
            let _ = draw(f, &s, &clock, "Compae · el asistente de garrafón");
        })
        .unwrap();

        let header = header_row(&term);
        assert!(header.contains(es().cfg_dirty), "the unsaved marker must be on screen:\n{header}");
        assert!(header.contains(es().cfg_title), "beside the screen's own name:\n{header}");
        assert!(header.contains('⚙'), "and the header's own cells are untouched:\n{header}");

        // A form with no edits has nothing unsaved, and the marker is not the header's to keep.
        s.config_form.as_mut().unwrap().dirty = false;
        term.draw(|f| {
            let _ = draw(f, &s, &clock, "Compae · el asistente de garrafón");
        })
        .unwrap();
        assert!(
            !header_row(&term).contains(es().cfg_dirty),
            "an untouched form must not be marked unsaved"
        );
    }
}
