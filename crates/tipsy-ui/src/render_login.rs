//! The login widget and the veil over the HUD behind it (the login screen design spec, §6 and §7).
//!
//! The screen is not a full-screen form: `draw` paints the whole HUD as it always does, [`veil`]
//! paints over everything outside the widget's rectangle, and the widget is drawn last, crisp, in the
//! middle. The core inside it is the HUD's own ([`crate::render::draw_core`]), drawn without the
//! HUD's "core" panel because the widget's border is its frame.
//!
//! Nothing here is authentication and nothing here is described as it: the screen is a ritual — a
//! flavour gate — and `server.token` remains the only real door.

use ratatui::Frame;
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use crate::render::{CARET, CYAN, DIM, GOLD, Hit, INK, cut_to, draw_core};
use crate::state::{KeyInput, LoginLine, LoginPhase, LoginState, UiState};

/// The widget's widest form: `min(48, area.width - 4)`.
const WIDGET_MAX_W: u16 = 48;
/// The widget at its full height: `min(13, area.height - 2)`.
const WIDGET_MAX_H: u16 = 13;
/// The rows the widget spends on its two borders, its status line and its input row. What is left
/// over is the core's, and the core needs at least three of them (spec §6).
const CHROME_H: u16 = 4;
/// The application's own name on the top border: the *application's* title, never the profile's. The
/// profile speaks from inside, and the badge it wears on the HUD's header is behind the veil, which is
/// why this is not a string-table entry.
const TITLE: &str = "◉ tipsy-assistant";
/// The marker the input row draws before what is typed.
const PROMPT: &str = "❯ ";
/// The columns [`PROMPT`] takes before the answer's first character.
const PROMPT_W: u16 = 2;
/// The ink of a veiled empty cell: near-black, so a frame of them is texture rather than a screenful
/// of holes (spec §7).
const VEIL_EMPTY: Color = Color::Rgb(40, 40, 40);

/// Which of the two veils [`veil`] paints (spec §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Veil {
    /// The default: an inked cell becomes `▒` in its own darkened colour, an empty one `░` in
    /// [`VEIL_EMPTY`].
    Blocks,
    /// The other extreme, for a font or a terminal where `▒`/`░` land badly: every cell keeps its own
    /// symbol and only takes the darkened colour, and a cell that was empty stays empty.
    ///
    /// Dead in a library-only build, and deliberately so: [`VEIL`] is [`Veil::Blocks`], and the only
    /// thing that constructs this variant is the veil's own test, which a plain `cargo build` does not
    /// compile. It is the alternate strength, kept one constant away and covered by that test.
    #[allow(dead_code)]
    Dim,
}

/// The strength this build ships with. [`Veil::Dim`] is one constant away (spec §7), and both
/// strengths are tested, so switching is not a rewrite.
pub(crate) const VEIL: Veil = Veil::Blocks;

/// The darkened form of one of the HUD's inks (spec §7's colour map).
///
/// The arms are named by colour rather than by the constant they came from — `CYAN` and `DIM` are the
/// same hue at two brightnesses, so they darken to the same ink — and a colour the map does not know
/// is kept as it is, which is what makes the veil safe over a palette that grows.
fn veil_fg(color: Color) -> Color {
    match color {
        CYAN | DIM => Color::Rgb(0, 80, 80),
        Color::Red => Color::Rgb(90, 30, 30),
        Color::Yellow | GOLD => Color::Rgb(90, 70, 30),
        Color::Green => Color::Rgb(30, 80, 30),
        Color::White => Color::Rgb(150, 150, 150),
        Color::Gray | Color::DarkGray => Color::Rgb(90, 90, 90),
        other => other,
    }
}

/// True for a cell that carries a symbol of its own, as opposed to one that was only ever given a
/// background: a cell the HUD never drew into is a space, whatever style it was left with.
fn inked(cell: &Cell) -> bool {
    !cell.symbol().trim().is_empty()
}

/// Rewrites every cell of `buf` outside `skip`, so that what the HUD drew behind the widget is
/// visibly there and visibly out of focus: an inked cell becomes `▒` in its own colour, darkened by
/// the map above, and an empty one becomes `░` in [`VEIL_EMPTY`]. Under [`Veil::Dim`] the symbols stay
/// and only the colour changes.
///
/// One pass over the frame's buffer per redraw (spec §7's cost note). It is a free function over a
/// [`Buffer`] rather than a widget on purpose: that is what makes both strengths a table of cells in
/// a test rather than a screenshot.
pub(crate) fn veil(buf: &mut Buffer, skip: Rect, strength: Veil) {
    let area = buf.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if skip.contains((x, y).into()) {
                // The widget's own cells: it is drawn after this, crisp, in the middle.
                continue;
            }
            // A colour that was never set has nothing to darken, and the map keeps it.
            let dark = veil_fg(buf[(x, y)].style().fg.unwrap_or(Color::Reset));
            let cell = &mut buf[(x, y)];
            match (strength, inked(cell)) {
                (Veil::Blocks, true) => {
                    cell.set_symbol("▒");
                    cell.set_fg(dark);
                }
                (Veil::Blocks, false) => {
                    cell.set_symbol("░");
                    cell.set_fg(VEIL_EMPTY);
                }
                (Veil::Dim, true) => {
                    cell.set_fg(dark);
                }
                (Veil::Dim, false) => {}
            }
        }
    }
}

/// The widget's rectangle on a frame of `area` (spec §6): centred, `min(48, width - 4)` by
/// `min(13, height - 2)`, shrunk to the four rows of its chrome when there is no room for the core.
///
/// `None` means the frame cannot hold a bordered widget at all — fewer than the 20 columns a border,
/// a status line and a readable shadow need, or fewer than 4 rows for the border, the status and the
/// input. The caller then draws the bare input row on the last line and reports no hit: the login is
/// never the thing that makes a tiny terminal unusable.
pub(crate) fn widget_rect(area: Rect) -> Option<Rect> {
    if area.width < 20 || area.height < CHROME_H {
        return None;
    }
    let w = WIDGET_MAX_W.min(area.width - 4);
    let h = WIDGET_MAX_H.min(area.height - 2);
    // `h - 4` rows of core: with fewer than three the core is dropped altogether and the widget is
    // its chrome, because a squashed core is worse than none. The arithmetic saturates, because a
    // frame only four rows tall reaches here with `h = 2`.
    //
    // The guard is belt and braces and says so: `area.height >= 4` gives `h >= 2` and the two are
    // independent, so `h` is never below 4 *when the frame is at least 6 rows tall* — which is the
    // range real terminals live in — and the branch below is reachable only from a frame with no room
    // for the core's three rows at all.
    let h = if h.saturating_sub(CHROME_H) < 3 { CHROME_H } else { h };
    Some(Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h))
}

/// Draws the login screen over the veiled frame and returns **only** the hits it drew (spec §6).
///
/// The veil is the first thing that happens here, over a buffer `draw` has already filled with the
/// whole HUD; the widget's rectangle is then cleared — so nothing of the dashboard shows through it —
/// and the widget is drawn into it. Everything behind the veil reports nothing, which is what keeps a
/// click on the dashboard from opening the configuration screen, clearing the conversation or
/// replaying a reply while the door is shut.
pub fn draw_login(frame: &mut Frame, state: &UiState, area: Rect) -> Vec<Hit> {
    let Some(login) = state.login.as_ref() else {
        return Vec::new();
    };
    let Some(widget) = widget_rect(area) else {
        // Spec §6's last resort: no border, no core, and no hit — the keys still work, because the
        // client routes them through `UiState` and never through a [`Hit`].
        let row = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
        let (input, cursor) = input_line(login, row.width.saturating_sub(1));
        frame.render_widget(Paragraph::new(Line::from(input)), row);
        if row.y < frame.area().bottom() {
            frame.set_cursor_position((row.x + cursor, row.y));
        }
        return Vec::new();
    };
    veil(frame.buffer_mut(), widget, VEIL);
    frame.render_widget(Clear, widget);
    let (title, core, status, input) = layout(widget);
    draw_chrome(frame, state, login, widget);
    if let Some(core) = core {
        draw_core(frame, state, core);
    }
    frame.render_widget(Paragraph::new(Line::from(status_line(state, login))), status);
    let limit = input.width.saturating_sub(1);
    let (spans, cursor) = input_line(login, limit);
    frame.render_widget(Paragraph::new(Line::from(spans)), input);
    if input.y < frame.area().bottom() {
        frame.set_cursor_position((input.x + cursor, input.y));
    }
    // Two controls, and only these two: the title answers `Tab` and the input row answers `Enter` —
    // a click is the keystroke its control already means, so neither carries an `action`.
    let title = title_area(frame, title);
    vec![Hit { area: title, key: KeyInput::Tab, action: None }, Hit { area: input, key: KeyInput::Enter, action: None }]
}

/// The widget's rows: its title border, the core when there is room for it, one status line and the
/// input row (spec §6).
fn layout(widget: Rect) -> (Rect, Option<Rect>, Rect, Rect) {
    let title = Rect::new(widget.x, widget.y, widget.width, 1);
    let inner = Rect::new(widget.x + 1, widget.y + 1, widget.width.saturating_sub(2), widget.height.saturating_sub(2));
    // The last two rows of the widget are the status line and the input, whatever is left above them.
    let input = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
    let status = Rect::new(inner.x, input.y.saturating_sub(1), inner.width, 1);
    let core_h = inner.height.saturating_sub(2);
    let core = (core_h >= 3).then(|| Rect::new(inner.x, inner.y, inner.width, core_h));
    (title, core, status, input)
}

/// The widget's border, in the widget's own ink: the application's name on its left and the recording
/// badge on its right while the capture is open, with the keys — or the escape hint, once the
/// automatic re-arm has given up — along the bottom (spec §6).
fn draw_chrome(frame: &mut Frame, state: &UiState, login: &LoginState, area: Rect) {
    let strings = state.strings();
    let mut titles = vec![cut_to(&format!(" {TITLE} "), Style::new().fg(CARET).bold(), area.width as usize)];
    // The screen's own capture is what `● rec` means, and it is the screen's own state — not the
    // HUD's `Tab` switch, which belongs to a microphone the veil is hiding and which the door may
    // find in either position (review finding 3). With the door shut there is exactly one microphone
    // that matters.
    if login.voice && login.open {
        // `● rec`, right-aligned and red, exactly while the microphone is open: the v1 rule about a
        // visible indicator, kept without a header.
        let rec = Span::from(format!(" {} ", strings.login_rec)).fg(Color::Red).bold();
        titles.push(rec);
    }
    let keys = if login.needs_a_key() { strings.login_keys_lost } else { strings.login_keys };
    let keys = cut_to(&format!(" {keys} "), Style::new().fg(DIM), area.width as usize);
    // `title` is the *top* border and `title_bottom` is the other one: on a widget this is the whole
    // difference between the keys riding the bottom border and never being drawn at all.
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(DIM))
        .title(Line::from(titles))
        .title_bottom(Line::from(keys));
    frame.render_widget(block, area);
}

/// The one status line, in the precedence of spec §6.1: the door opening, the refusal, what was
/// heard, a failure, the transcription, and then the prompt. Typing never clears a refusal: the input
/// box and this row are different rows, and reading the refusal while typing the next try is the
/// point.
fn status_line(state: &UiState, login: &LoginState) -> Vec<Span<'static>> {
    let strings = state.strings();
    let text = |text: String, style: Style| vec![Span::styled(text, style)];
    match (&login.line, login.phase) {
        (_, LoginPhase::Passed) => text(pass_string(state, login), Style::new().fg(Color::Green).bold()),
        (LoginLine::Failed(message), _) => text(message.clone(), Style::new().fg(Color::Red)),
        (LoginLine::Refused, _) => text(strings.login_refusal.to_string(), Style::new().fg(GOLD).bold()),
        (LoginLine::NothingHeard, _) => text(strings.login_nothing_heard.to_string(), Style::new().fg(GOLD).bold()),
        (LoginLine::Heard(heard), _) => text(strings.login_heard.replace("{text}", heard), Style::new().fg(CYAN)),
        // An answer is with the transcriber: the same word the HUD uses for the wait, in the same
        // ink the HUD's own spinner is.
        (LoginLine::Transcribing, _) => text(strings.transcribing.to_string(), Style::new().fg(CYAN)),
        // `Waiting` with nothing to report: the prompt.
        _ => text(strings.login_prompt.to_string(), Style::new().fg(INK)),
    }
}

/// What the door says once it opens: the matched entry's name, or the nameless greeting when the
/// entry carries none (spec §6.1).
fn pass_string(state: &UiState, login: &LoginState) -> String {
    let strings = state.strings();
    match login.name.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => strings.login_pass.replace("{name}", name),
        _ => strings.login_pass_anon.to_string(),
    }
}

/// The input row: the caret, then the answer being typed, cut to the room it has. The cursor is the
/// column the next character lands in, in the same characters-not-columns reckoning the HUD's own
/// question box uses.
fn input_line(login: &LoginState, limit: u16) -> (Vec<Span<'static>>, u16) {
    let caret = Span::from(PROMPT).fg(CARET);
    let text = cut_to(&login.input, Style::new().fg(INK), limit.saturating_sub(PROMPT_W) as usize);
    let cursor = (PROMPT_W + login.input.chars().count() as u16).min(limit);
    (vec![caret, text], cursor)
}

/// The rectangle the title occupies on the top border.
///
/// Ratatui does not report where it laid a title out, but it does draw it where it was asked to: the
/// block's left title starts one column inside the corner — the corner itself is the border — and the
/// title is drawn with the spaces that pad it. The row is therefore read from its own two ends: the
/// corner marks where the title may start, the first border character after it marks where it must
/// stop, and the trailing padding inside those two is not part of the control.
fn title_area(frame: &mut Frame, area: Rect) -> Rect {
    let (x0, x1, y) = (area.x, area.right(), area.y);
    let buffer = frame.buffer_mut();
    let row: String = (x0 + 1..x1)
        .take_while(|&x| !matches!(buffer[(x, y)].symbol(), "╮" | "╯" | "│" | "─"))
        .map(|x| buffer[(x, y)].symbol())
        .collect();
    let width = row.trim_end().chars().count() as u16;
    Rect::new(x0 + 1, y, width.min(area.width), 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(width: u16, height: u16) -> Rect {
        Rect::new(0, 0, width, height)
    }

    #[test]
    fn widget_rect_centres_the_widget_and_keeps_it_inside_the_frame() {
        // 48 x 13 inside a 150 x 40 frame: one column off centre, six rows off centre.
        assert_eq!(widget_rect(area(150, 40)), Some(Rect::new(51, 13, 48, 13)));
        // The frame is the limit, not the wanted size: a 60 x 16 frame holds 48 x 13 at (6, 1).
        assert_eq!(widget_rect(area(60, 16)), Some(Rect::new(6, 1, 48, 13)));
        // Narrower than it wants, still centred.
        assert_eq!(widget_rect(area(40, 20)), Some(Rect::new(2, 3, 36, 13)));
    }

    #[test]
    fn widget_rect_gives_up_rows_before_it_gives_up_the_core() {
        // `h = min(13, height - 2)`, so the core loses a row for every row the frame does: at 12 rows
        // the widget is 10 rows — six of core — and at the 9 a 11-row frame holds it is 7.
        assert_eq!(widget_rect(area(60, 12)), Some(Rect::new(6, 1, 48, 10)));
        assert_eq!(widget_rect(area(60, 11)), Some(Rect::new(6, 1, 48, 9)));
        // The core never gets fewer than the three rows it needs, and the widget is never taller than
        // its ceiling: `min(13, height - 2)` with `height >= 6` can never fall into the chrome-only
        // branch, which is why the widget keeps its core at every size a real terminal has.
        for height in CHROME_H..40 {
            let rect = widget_rect(area(60, height)).expect("a frame this size holds a widget");
            assert!(rect.height <= WIDGET_MAX_H, "{height}: {}", rect.height);
            assert!(rect.y + rect.height <= height, "{height}: {rect:?} hangs off the frame");
        }
    }

    #[test]
    fn widget_rect_is_none_when_the_frame_cannot_border_a_widget() {
        assert_eq!(widget_rect(area(19, 20)), None, "no room for the 16 columns a widget needs");
        assert_eq!(widget_rect(area(20, 3)), None, "no room for the border, a status and an input");
        assert_eq!(widget_rect(area(10, 2)), None);
        // The two minima are inclusive: 20 x 4 is the smallest frame that holds one, and at four rows
        // the widget is exactly its chrome — there is no core to draw, and no underflow either.
        assert_eq!(widget_rect(area(20, 4)), Some(Rect::new(2, 0, 16, 4)));
    }

    /// The widget's own cell, inside the rectangle `veil` is told to skip: the cell the two tests
    /// below read to prove the widget was left alone.
    const WIDGET: Rect = Rect { x: 4, y: 3, width: 6, height: 4 };

    /// A buffer with one inked cell outside the widget, one inked cell inside it and one empty cell
    /// outside it.
    fn fixture() -> Buffer {
        let mut buf = Buffer::empty(area(12, 8));
        buf[(0, 0)].set_symbol("A");
        buf[(0, 0)].set_fg(Color::Rgb(0, 210, 255));
        buf[(WIDGET.x, WIDGET.y)].set_symbol("W");
        buf[(WIDGET.x, WIDGET.y)].set_fg(Color::White);
        buf
    }

    #[test]
    fn the_veil_blocks_every_cell_outside_the_widget_and_leaves_its_own_alone() {
        let mut buf = fixture();
        veil(&mut buf, WIDGET, Veil::Blocks);

        // An inked cell keeps its own (darkened) colour and becomes a block.
        assert_eq!(buf[(0, 0)].symbol(), "▒", "an inked cell becomes the block");
        assert_eq!(buf[(0, 0)].style().fg, Some(Color::Rgb(0, 80, 80)), "cyan, darkened");
        // An empty one becomes the lighter block in the near-black ink.
        assert_eq!(buf[(1, 0)].symbol(), "░");
        assert_eq!(buf[(1, 0)].style().fg, Some(VEIL_EMPTY));
        // And the widget's own cells were skipped: the widget is drawn after the veil and is the only
        // crisp thing on screen.
        assert_eq!(buf[(WIDGET.x, WIDGET.y)].symbol(), "W");
        assert_eq!(buf[(WIDGET.x, WIDGET.y)].style().fg, Some(Color::White));
    }

    #[test]
    fn the_dim_veil_keeps_the_symbols_and_only_darkens() {
        let mut buf = fixture();
        veil(&mut buf, WIDGET, Veil::Dim);

        assert_eq!(buf[(0, 0)].symbol(), "A", "the symbol is the HUD's own again");
        assert_eq!(buf[(0, 0)].style().fg, Some(Color::Rgb(0, 80, 80)));
        assert_eq!(buf[(1, 0)].symbol(), " ", "a cell that was empty stays empty");
        assert_eq!(buf[(WIDGET.x, WIDGET.y)].symbol(), "W");
    }

    #[test]
    fn the_veil_knows_the_palette_and_keeps_a_colour_it_does_not() {
        let cases: &[(Color, Color)] = &[
            (CYAN, Color::Rgb(0, 80, 80)),
            (DIM, Color::Rgb(0, 80, 80)),
            (GOLD, Color::Rgb(90, 70, 30)),
            (Color::Red, Color::Rgb(90, 30, 30)),
            (Color::Yellow, Color::Rgb(90, 70, 30)),
            (Color::Green, Color::Rgb(30, 80, 30)),
            (Color::White, Color::Rgb(150, 150, 150)),
            (Color::Gray, Color::Rgb(90, 90, 90)),
            (Color::DarkGray, Color::Rgb(90, 90, 90)),
            (Color::Magenta, Color::Magenta),
            (Color::Rgb(1, 2, 3), Color::Rgb(1, 2, 3)),
        ];
        for (from, to) in cases {
            assert_eq!(veil_fg(*from), *to, "{from:?}");
        }
    }
}
