//! HUD drawing.
//!
//! A port of the proof of concept HUD (`poc/src/main.rs`) onto [`UiState`]: the clock, the system
//! figures and the audio envelopes all arrive through the state, so nothing here reads a clock or
//! probes the machine.

use std::f64::consts::TAU;

use ratatui::Frame;
use ratatui::layout::Constraint::{Length, Min, Percentage};
use ratatui::layout::{Alignment, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Circle, Context, Line as CLine, Points};
use ratatui::widgets::{
    Axis, Block, BorderType, Chart, Dataset, Gauge, GraphType, LegendPosition, List, ListItem, ListState, Paragraph,
    Sparkline,
};
use tipsy_protocol::{AssistantState, UiLanguage};

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
    pub key: KeyInput,
}

pub fn draw(frame: &mut Frame, state: &UiState, clock: &ClockView, title: &str) -> Vec<Hit> {
    let [header, body, input, help] = Layout::vertical([Length(1), Min(10), Length(3), Length(1)]).areas(frame.area());
    let [left, center, right] = Layout::horizontal([Percentage(LEFT_PERCENT), Min(30), Percentage(26)]).areas(body);

    draw_header(frame, state, header, title);

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

    hits
}

/// Badge with the persona's title, then whatever the assistant is doing.
fn draw_header(frame: &mut Frame, state: &UiState, area: Rect, title: &str) {
    let strings = state.strings();
    let spin = SPINNER[(state.elapsed_ms / 150) as usize % SPINNER.len()];
    let status: Span = if state.mic_open {
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
    frame.render_widget(Line::from(vec![badge, Span::from("  "), status]), area);
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
        Hit { area: row(area.x + 2, chat_w), key: KeyInput::F(7) },
        Hit { area: row(area.x + 2 + chat_w + 1, net_w), key: KeyInput::F(8) },
        Hit { area: row(area.right().saturating_sub(1 + clear_w), clear_w), key: KeyInput::F(4) },
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
}
