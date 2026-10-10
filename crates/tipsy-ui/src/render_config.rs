//! The configuration screen: the four tabs of [`ConfigForm`], drawn from [`ConfigForm::rows`].
//!
//! One row list feeds both the drawing and the click areas, so what is on screen and what a click
//! can reach cannot disagree. The screen is drawn as a whole frame (spec §9: full screen), and every
//! rectangle is clipped before it is used, so a terminal near the layout's minimum stays readable
//! instead of overlapping itself.

use ratatui::Frame;
use ratatui::layout::Constraint::{Length, Min};
use ratatui::layout::{Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::config_form::{ConfigForm, ConfigTab, FormRow, NoticeKind};
use crate::render::Hit;
use crate::state::KeyInput;
use crate::strings::Strings;

const CYAN: Color = Color::Rgb(0, 210, 255);
const DIM: Color = Color::Rgb(0, 95, 125);
const GOLD: Color = Color::Rgb(255, 190, 60);
/// The bar the focused row is drawn on: dark enough that the label and the value keep their own
/// colours on top of it, the same dark teal the HUD's gauges use.
const BAR: Color = Color::Rgb(0, 30, 40);
/// The `F` keys the screen's tabs answer to: the HUD already spends `F7` and `F8` on chat/network,
/// so the screen's four tabs take the four after them.
const TAB_KEYS: [u8; 4] = [7, 8, 9, 10];
/// The screen's two actions. `F2` and `F4` are the HUD's replay and clear, and the screen reuses
/// neither while it is open.
const TEST_KEY_F: u8 = 2;
const SAVE_KEY_F: u8 = 4;
/// The label column: a third of the body, wide enough for "Instrucciones de voz" and narrow enough
/// that the value column still has room on a small terminal.
const LABEL_MIN: u16 = 12;
const LABEL_MAX: u16 = 26;
/// The four tabs, in the order [`TAB_KEYS`] names their keys — the order `ConfigForm`'s own tab
/// switching walks, so a click and a key reach the same tab.
const TABS: [ConfigTab; 4] =
    [ConfigTab::Providers, ConfigTab::Listening, ConfigTab::Profiles, ConfigTab::Server];
/// The columns between the two buttons of the action line.
const ACTION_GAP: u16 = 2;
/// The edit cursor: a thin bar drawn between the committed value and the buffer being typed.
const CURSOR: &str = "│";

/// Draws the configuration screen over the whole frame and reports every clickable area: the four
/// tab labels, then the rows of [`ConfigForm::rows`] in order, then the Test and the Save button.
pub fn draw_form(frame: &mut Frame, form: &ConfigForm, strings: &Strings, elapsed_ms: u64) -> Vec<Hit> {
    // The frame's tick: the client hands the screen the same `elapsed_ms` the HUD animates with, so
    // the signature does not change when something here starts to move. Nothing on this screen
    // animates yet, so the tick is accepted and not read — and no wall clock is read here either.
    let _ = elapsed_ms;
    let [title, tabs, notice, body, help] =
        Layout::vertical([Length(1), Length(1), Length(1), Min(3), Length(1)]).areas(frame.area());

    // The layout squeezes a frame that cannot hold all five rows by taking lines off its front, so
    // the body is only the body when the head is whole: the rows are drawn at `body.y`, which has to
    // be the line under the notice. A frame too short for that draws whatever fits and reports
    // nothing, rather than a click area nobody can see on a line the layout never promised.
    let body = if title.height == 1 && tabs.height == 1 && notice.height == 1 { body } else { Rect::ZERO };

    let mut hits = draw_tabs(frame, form, strings, tabs);
    draw_title(frame, form, strings, title);
    draw_notice(frame, form, notice);
    hits.extend(draw_body(frame, form, strings, body));
    draw_help(frame, form, strings, help);
    hits
}

/// The screen's name, and `cfg_dirty` beside it while there is something unsaved. The dirty marker
/// is part of the title rather than a notice, so a notice never has to say it.
fn draw_title(frame: &mut Frame, form: &ConfigForm, strings: &Strings, area: Rect) {
    if area.is_empty() {
        return;
    }
    let mut runs = vec![(format!(" {} ", strings.cfg_title), Style::new().fg(CYAN).bold())];
    if form.dirty {
        runs.push((strings.cfg_dirty.to_string(), Style::new().fg(GOLD)));
    }
    frame.render_widget(Paragraph::new(cut_line(runs, area.width as usize)), area);
}

/// The tab row: the four labels, one space between them, the active one on a cyan bar and the row
/// starting one column in. Each hit is the label's own cells, measured with `Span::width` the way
/// `render.rs` measures the chat/network tabs; a label the frame has already run past is not drawn,
/// so it reports nothing rather than half a rectangle.
fn draw_tabs(frame: &mut Frame, form: &ConfigForm, strings: &Strings, area: Rect) -> Vec<Hit> {
    if area.is_empty() {
        return Vec::new();
    }
    let mut runs = vec![(" ".to_string(), Style::default())];
    let mut hits = Vec::new();
    let mut x = area.x.saturating_add(1);
    for (index, tab) in TABS.iter().enumerate() {
        let label = form.tab_title(*tab, strings);
        let width = text_width(label) as u16;
        let style = if *tab == form.tab {
            Style::new().fg(Color::Black).bg(CYAN).bold()
        } else {
            Style::new().fg(DIM)
        };
        if width > 0 && x.saturating_add(width) <= area.right() {
            hits.push(Hit { area: Rect::new(x, area.y, width, 1), key: KeyInput::F(TAB_KEYS[index]) });
        }
        runs.push((label.to_string(), style));
        runs.push((" ".to_string(), Style::default()));
        x = x.saturating_add(width).saturating_add(1);
    }
    frame.render_widget(Paragraph::new(cut_line(runs, area.width as usize)), area);
    hits
}

/// The notice line, coloured by [`NoticeKind`]. It is one row whatever it says — and still one row
/// when it says nothing — so the body never moves up and down under the user as notices come and go.
fn draw_notice(frame: &mut Frame, form: &ConfigForm, area: Rect) {
    if area.is_empty() {
        return;
    }
    let line = match &form.notice {
        Some((kind, text)) => Line::from(cut(text, area.width as usize, false)).fg(notice_colour(kind)),
        None => Line::default(),
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn notice_colour(kind: &NoticeKind) -> Color {
    match kind {
        NoticeKind::Ok => Color::Green,
        NoticeKind::Warning => GOLD,
        NoticeKind::Error => Color::Red,
    }
}

/// The help line: the editing keys while a buffer is open, the screen's keys otherwise. It is drawn
/// at the row's second column because the table's help line carries its own leading space.
fn draw_help(frame: &mut Frame, form: &ConfigForm, strings: &Strings, area: Rect) {
    if area.is_empty() {
        return;
    }
    let inner = Rect { x: area.x.saturating_add(1), width: area.width.saturating_sub(1), ..area };
    if inner.is_empty() {
        return;
    }
    // A row being edited has its own two keys and they are the ones that matter while the buffer is
    // open: the whole screen's list does not fit beside them at the width the tests pin, and a help
    // line cut in the middle of a word tells nobody anything.
    let help = if form.editing.is_some() { strings.cfg_help_editing } else { strings.cfg_help };
    frame.render_widget(Paragraph::new(cut_line(
        vec![(help.to_string(), Style::new().fg(DIM))],
        inner.width as usize,
    )), inner);
}

/// The body: the rows of [`ConfigForm::rows`], and the action line under them. The rows are drawn on
/// the body's first lines, so a row's `y` is the body's `y` plus its index; the rows past the last
/// one the body holds are not drawn and report no hit, so a click cannot reach a row nobody can see.
fn draw_body(frame: &mut Frame, form: &ConfigForm, strings: &Strings, body: Rect) -> Vec<Hit> {
    if body.is_empty() {
        return Vec::new();
    }
    // The label column: a third of the body, never so wide that the value column has less than four
    // cells to show anything in.
    let label_w = (body.width / 3).clamp(LABEL_MIN, LABEL_MAX).min(body.width.saturating_sub(4));
    let value_x = body.x.saturating_add(label_w);
    // The action line is the body's last row. A body of one row is all rows and no buttons: a row
    // the user can read is worth more than a button that would push it off the frame.
    let (rows_area, actions) = if body.height >= 2 {
        (
            Rect { height: body.height - 1, ..body },
            Rect { x: body.x, y: body.bottom() - 1, width: body.width, height: 1 },
        )
    } else {
        (body, Rect::ZERO)
    };

    let mut hits = Vec::new();
    for (index, row) in form.rows().iter().enumerate() {
        let y = rows_area.y.saturating_add(index as u16);
        if y >= rows_area.bottom() {
            break;
        }
        let value = Rect::new(value_x, y, body.right().saturating_sub(value_x), 1);
        if let Some(hit) = draw_row(frame, form, row, index, label_w, value) {
            hits.push(hit);
        }
    }
    hits.extend(draw_actions(frame, strings, actions));
    hits
}

/// One row: the label in the left column, the value in the right one, both on the focus bar when the
/// row is focused. The row's click area is its value column, clipped to the body's right edge, so a
/// click on the column's right edge belongs to the row whose column it is. The value is cut at that
/// edge and never spilled past it: a long base URL stops where the next column begins.
fn draw_row(
    frame: &mut Frame,
    form: &ConfigForm,
    row: &FormRow,
    index: usize,
    label_w: u16,
    value: Rect,
) -> Option<Hit> {
    if value.is_empty() {
        return None;
    }
    let focused = index == form.field;
    let bar = if focused { Style::new().bg(BAR) } else { Style::new() };
    let label_area = Rect { x: value.x.saturating_sub(label_w), y: value.y, width: label_w, height: 1 };
    let label =
        Span::styled(cut(row.label, label_area.width as usize, false), Style::new().fg(if focused { CYAN } else { DIM }));
    frame.render_widget(Paragraph::new(Line::from(vec![label])).style(bar), label_area);

    let width = value.width as usize;
    let mut spans = Vec::new();
    if focused && form.editing.is_some() {
        // The committed value stays on screen while the buffer is typed over it — the buffer
        // *replaces* it on commit, so what is about to go is dimmed rather than hidden. One column
        // is reserved for the cursor, which therefore sits at the column's last cell whatever the
        // value's length, and the value is cut from the left so its end stays beside the cursor.
        let value_text = cut(&row.value, width.saturating_sub(1), true);
        let used = text_width(&value_text);
        spans.push(Span::styled(value_text, Style::new().fg(Color::White)));
        spans.push(Span::from(CURSOR).fg(CYAN));
        let buffer = form.editing.as_deref().unwrap_or_default();
        spans.push(Span::styled(cut(buffer, width.saturating_sub(used + 1), false), Style::new().fg(DIM)));
    } else {
        // The marker an active profile carries is in the row's own value (`◉`); the colour only
        // repeats it, and the focus bar wins where the two meet.
        let fg = if !focused && row.is_active { GOLD } else { Color::White };
        spans.push(Span::styled(cut(&row.value, width, false), Style::new().fg(fg)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(bar), value);
    Some(Hit { area: value, key: KeyInput::Enter })
}

/// The action line: the Test button and the Save button, each a padded label on a bar and each
/// reporting the cells of its own label. The words are the table's, the same two the row list uses
/// for its Test and Save rows, so the screen has no third name for either action.
fn draw_actions(frame: &mut Frame, strings: &Strings, area: Rect) -> Vec<Hit> {
    if area.is_empty() {
        return Vec::new();
    }
    let mut runs = Vec::new();
    let mut hits = Vec::new();
    let mut x = area.x;
    for (label, colour, key) in [
        (strings.cfg_test, GOLD, KeyInput::F(TEST_KEY_F)),
        (strings.cfg_save, CYAN, KeyInput::F(SAVE_KEY_F)),
    ] {
        let button = format!(" {label} ");
        let button_w = text_width(&button) as u16;
        // Half a button is not a button: one that does not fit the frame is not drawn and reports
        // nothing, rather than reporting cells whose label is cut off.
        if x.saturating_add(button_w) > area.right() {
            break;
        }
        hits.push(Hit {
            area: Rect::new(x.saturating_add(1), area.y, text_width(label) as u16, 1),
            key,
        });
        runs.push((button, Style::new().fg(Color::Black).bg(colour).bold()));
        runs.push((" ".repeat(ACTION_GAP as usize), Style::default()));
        x = x.saturating_add(button_w).saturating_add(ACTION_GAP);
    }
    frame.render_widget(Paragraph::new(cut_line(runs, area.width as usize)), area);
    hits
}

/// The columns `text` takes on screen, by ratatui's own reckoning.
fn text_width(text: &str) -> usize {
    Span::from(text).width()
}

/// `text` cut to at most `width` columns: the characters past the edge are dropped, never split, and
/// the cut counts column widths rather than bytes. `keep_end` keeps the end of the string instead of
/// its start — what a row being edited needs, whose cursor sits at the end of the buffer. At least
/// one character is always kept, so a symbol wider than the room it has cannot loop for ever; this
/// is the rule `render::cut_at` already uses.
fn cut(text: &str, width: usize, keep_end: bool) -> String {
    let width = width.max(1);
    let mut used = 0;
    let mut end = 0;
    let mut start = text.len();
    for (i, c) in text.char_indices() {
        let w = text_width(c.encode_utf8(&mut [0; 4]));
        if used > 0 && used + w > width {
            break;
        }
        used += w;
        end = i + c.len_utf8();
    }
    if !keep_end {
        return text[..end].to_string();
    }
    used = 0;
    for (i, c) in text.char_indices().rev() {
        let w = text_width(c.encode_utf8(&mut [0; 4]));
        if used > 0 && used + w > width {
            break;
        }
        used += w;
        start = i;
    }
    text[start..].to_string()
}

/// Styled runs cut to `width` columns. Every line this file builds goes through here — the title,
/// the tab row, the action line and the help line — because a run wider than the rectangle it is
/// drawn in would spill into the cells a neighbouring widget is about to write.
fn cut_line(runs: Vec<(String, Style)>, width: usize) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut left = width;
    for (text, style) in runs {
        if left == 0 {
            break;
        }
        let full = text_width(&text);
        let cut = cut(&text, left, false);
        let cut_w = text_width(&cut);
        if !cut.is_empty() {
            spans.push(Span::styled(cut, style));
        }
        left = left.saturating_sub(cut_w);
        if cut_w < full {
            break;
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_form::{ConfigTab, ListeningForm, NoticeKind, ProfileForm, ProviderForm, ServerForm};
    use crate::strings::strings;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use tipsy_protocol::{Capability, ListeningMode, UiLanguage};

    /// A frame's worth of symbols, exactly as `crates/tipsy-ui/tests/render.rs` reads them.
    fn rendered(form: &ConfigForm, lang: UiLanguage, w: u16, h: u16) -> (String, Vec<Hit>, Terminal<TestBackend>) {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Vec::new();
        term.draw(|f| hits = draw_form(f, form, strings(lang), 0)).unwrap();
        let text: String = term.backend().buffer().content.iter().map(|c| c.symbol()).collect();
        (text, hits, term)
    }

    /// The symbols of one rectangle, as `tests/render.rs`'s `text_at` reads them.
    fn text_at(term: &Terminal<TestBackend>, r: Rect) -> String {
        (r.x..r.right()).map(|x| term.backend().buffer()[(x, r.y)].symbol().to_string()).collect()
    }

    fn providers() -> ConfigForm {
        // Kept as one expression rather than assigned into a `ConfigForm::default()`: the branch's
        // clippy run flags `field_reassign_with_default`, and a fixture has no business adding one.
        ConfigForm {
            providers: vec![
                ProviderForm {
                    capability: Capability::Chat,
                    kind: "openai_compat".into(),
                    base_url: "https://openrouter.ai/api/v1".into(),
                    model: "google/gemini-2.5-flash".into(),
                    key_name: "openrouter".into(),
                },
                ProviderForm {
                    capability: Capability::Speech,
                    kind: "openai_compat".into(),
                    base_url: "https://openrouter.ai/api/v1".into(),
                    model: "google/gemini-3.8-flash-tts".into(),
                    key_name: "openrouter".into(),
                },
                ProviderForm {
                    capability: Capability::Transcribe,
                    kind: "openai_compat".into(),
                    base_url: "https://openrouter.ai/api/v1".into(),
                    model: "openai/whisper-large-v3".into(),
                    key_name: "openrouter".into(),
                },
            ],
            ..ConfigForm::default()
        }
    }

    /// The four tabs, each with a value that must not be mistaken for another tab's.
    fn tabs() -> Vec<(ConfigForm, &'static str)> {
        let es = strings(UiLanguage::Es);

        let mut listening = providers();
        listening.tab = ConfigTab::Listening;
        listening.listening = ListeningForm {
            mode: ListeningMode::Auto,
            silence_ms: "1500".into(),
            mic_gain: "9.5".into(),
            mic_device: Some("Rode NT-USB".into()),
        };

        let mut profiles = providers();
        profiles.tab = ConfigTab::Profiles;
        profiles.active_profile = "compae".into();
        profiles.profiles = vec![
            ProfileForm {
                id: "compae".into(),
                name: "El Compae".into(),
                language: "es-ES".into(),
                prompt: "p".into(),
                voice_instructions: "v".into(),
                greeting: "g".into(),
                wake_word: "oye compae".into(),
                voices: vec![("gemini".into(), "Charon".into())],
            },
            ProfileForm {
                id: "meigo".into(),
                name: "O Meigo".into(),
                language: "es-ES".into(),
                prompt: "p".into(),
                voice_instructions: "v".into(),
                greeting: "g".into(),
                wake_word: "oye meigo".into(),
                voices: vec![("gemini".into(), "Puck".into())],
            },
        ];

        let mut server = providers();
        server.tab = ConfigTab::Server;
        server.server = ServerForm { bind: "0.0.0.0:7070".into(), lan: true, token: "-".repeat(70) };

        vec![
            (providers(), es.cfg_connector),
            (listening, es.cfg_mode),
            (profiles, es.cfg_wake_word),
            (server, es.cfg_bind),
        ]
    }

    /// The rect of the row whose label is `label`, off the drawn buffer: a label is drawn at the
    /// left of its row, on the label column, so its `y` is the row's `y`.
    fn row_at(term: &Terminal<TestBackend>, label: &str) -> Rect {
        let buffer = term.backend().buffer();
        let area = buffer.area;
        for y in area.top()..area.bottom() {
            let text: String =
                (area.left()..area.right()).map(|x| buffer[(x, y)].symbol().to_string()).collect();
            if let Some(at) = text.find(label) {
                return Rect::new(area.left() + at as u16, y, label.chars().count() as u16, 1);
            }
        }
        panic!("no row labelled {label:?} in the drawn buffer");
    }

    #[test]
    fn every_tab_draws_its_title_its_rows_and_the_help_line() {
        // Every row of every tab, at a width that holds all of them, in both languages.
        let mut all: Vec<&'static str> = Vec::new();
        for lang in [UiLanguage::Es, UiLanguage::En] {
            let s = strings(lang);
            assert!(!s.cfg_title.is_empty());
            for (f, _) in tabs() {
                let (text, _, _) = rendered(&f, lang, 120, 40);
                assert!(text.contains(s.cfg_title), "the screen's name is missing in {lang:?}:\n{text}");
                assert!(text.contains(f.tab_title(f.tab, s)), "no tab title for {:?}", f.tab);
                assert!(text.contains(s.cfg_test), "the Test button is missing:\n{text}");
                assert!(text.contains(s.cfg_save), "the Save button is missing:\n{text}");
                assert!(text.contains(s.cfg_help), "the help line is missing:\n{text}");
                for row in f.rows() {
                    assert!(text.contains(row.label), "the label {:?} is not drawn:\n{text}", row.label);
                    assert!(text.contains(&row.value), "the value {:?} is not drawn:\n{text}", row.value);
                    if !all.contains(&row.label) {
                        all.push(row.label);
                    }
                }
            }
        }
        // The four tabs together are what makes every label in the table reachable, so a label that
        // no tab can draw is a label nothing checks.
        let es = strings(UiLanguage::Es);
        for label in [
            es.cfg_connector,
            es.cfg_base_url,
            es.cfg_model,
            es.cfg_key_name,
            es.cfg_mode,
            es.cfg_silence_ms,
            es.cfg_mic_gain,
            es.cfg_mic_device,
            es.cfg_name,
            es.cfg_language,
            es.cfg_prompt,
            es.cfg_voice_instructions,
            es.cfg_greeting,
            es.cfg_wake_word,
            es.cfg_voices,
            es.cfg_bind,
            es.cfg_lan,
            es.cfg_save,
        ] {
            assert!(all.contains(&label), "no tab draws the {label:?} row");
        }
    }

    #[test]
    fn the_focused_row_is_marked_and_only_it_is() {
        let mut f = providers();
        f.field = 1;
        let (text, _, term) = rendered(&f, UiLanguage::Es, 120, 40);
        let row = row_at(&term, strings(UiLanguage::Es).cfg_base_url);
        let focused = term.backend().buffer()[(row.x, row.y)].style().bg;
        assert!(focused.is_some(), "the focused row is drawn on a bar:\n{text}");
        let other = row_at(&term, strings(UiLanguage::Es).cfg_model);
        assert_ne!(term.backend().buffer()[(other.x, other.y)].style().bg, focused);
    }

    #[test]
    fn the_active_profile_is_marked_in_the_list() {
        let mut f = providers();
        f.tab = ConfigTab::Profiles;
        f.active_profile = "meigo".into();
        f.profiles = vec![
            ProfileForm { id: "compae".into(), name: "El Compae".into(), ..ProfileForm::default() },
            ProfileForm { id: "meigo".into(), name: "O Meigo".into(), ..ProfileForm::default() },
        ];
        let (text, _, _) = rendered(&f, UiLanguage::Es, 120, 40);
        assert!(text.contains("◉ O Meigo") || text.contains("● O Meigo"), "the active profile is marked:\n{text}");
        assert!(!text.contains("◉ El Compae") && !text.contains("● El Compae"), "and only it is:\n{text}");
    }

    #[test]
    fn the_edited_row_shows_its_value_and_the_cursor_line() {
        let mut f = providers();
        f.field = 1;
        f.editing = Some(String::new());
        let (text, _, _) = rendered(&f, UiLanguage::Es, 120, 40);
        assert!(text.contains("https://openrouter.ai/api/v1")
            || text.contains("https://openrouter.ai/api/v"), "the value stays on screen while it is edited:\n{text}");
        assert!(text.contains('│'), "the cursor is drawn:\n{text}");
        assert!(text.contains(strings(UiLanguage::Es).cfg_help_editing), "and the help line says what Enter does:\n{text}");

        f.editing = Some("con".into());
        let (text, _, _) = rendered(&f, UiLanguage::Es, 120, 40);
        assert!(text.contains("con"), "what has been typed is on screen:\n{text}");
    }

    #[test]
    fn the_notice_line_is_drawn_and_carries_the_notice_kind() {
        for (kind, notice) in [
            (NoticeKind::Warning, "número no válido"),
            (NoticeKind::Error, "no se pudo escribir el archivo"),
            (NoticeKind::Ok, "guardado"),
        ] {
            let mut f = providers();
            f.notice = Some((kind, notice.into()));
            let (text, _, _) = rendered(&f, UiLanguage::Es, 120, 40);
            assert!(text.contains(notice), "the notice is missing:\n{text}");
        }
        // With nothing to say, the notice row is still a row: the fixed layout is what keeps the
        // body from moving, and the rows below it are the same rows either way.
        let bare = providers();
        let with_notice = {
            let mut f = providers();
            f.notice = Some((NoticeKind::Warning, "x".into()));
            f
        };
        let a = rendered(&bare, UiLanguage::Es, 120, 40).0;
        let b = rendered(&with_notice, UiLanguage::Es, 120, 40).0;
        assert!(a.contains(strings(UiLanguage::Es).cfg_help) && b.contains(strings(UiLanguage::Es).cfg_help));
        assert!(a.contains(strings(UiLanguage::Es).cfg_connector) && b.contains(strings(UiLanguage::Es).cfg_connector));
        // The two buffers are the same size, so an unchanged row is at an unchanged offset: the
        // notice took a row of its own rather than pushing the body down.
        let at = |text: &str, needle: &str| text.find(needle).unwrap();
        assert_eq!(
            at(&a, strings(UiLanguage::Es).cfg_connector) / 120,
            at(&b, strings(UiLanguage::Es).cfg_connector) / 120,
            "the first row is on the same line with and without a notice"
        );
    }

    #[test]
    fn every_clickable_area_sits_on_its_label() {
        let f = providers();
        let (_, hits, term) = rendered(&f, UiLanguage::Es, 120, 40);
        let labels: Vec<String> = hits.iter().map(|h| text_at(&term, h.area)).collect();
        // The screen's own controls, in the order `draw_form` reports them. The four tab labels are
        // the ones `ConfigForm::tab_title` names; the two buttons are the two last hits.
        let es = strings(UiLanguage::Es);
        assert_eq!(labels[0], es.cfg_providers);
        assert_eq!(labels[1], es.cfg_listening);
        assert_eq!(labels[2], es.cfg_profiles);
        assert_eq!(labels[3], es.cfg_server);
        assert_eq!(labels[hits.len() - 2], es.cfg_test);
        assert_eq!(labels[hits.len() - 1], es.cfg_save);
        // And the row hits are one per drawn row, on the row they name.
        let rows = f.rows();
        assert_eq!(hits.len() - 6, rows.len(), "every row has exactly one hit: {} hits, {} rows", hits.len(), rows.len());
    }

    #[test]
    fn a_value_too_long_for_its_column_is_cut_and_never_spills() {
        let mut f = providers();
        // "Instrucciones de voz" is the longest label there is, and a 70-character base URL cannot
        // fit beside it on a 60-column terminal.
        f.providers[0].base_url = "x".repeat(70);
        f.tab = ConfigTab::Listening;
        f.field = 0;
        let (_, hits, term) = rendered(&f, UiLanguage::Es, 60, 20);
        let row = hits.iter().find(|h| h.key == KeyInput::Enter && h.area.y > 3).expect("a row hit");
        let drawn = text_at(&term, row.area);
        assert!(!drawn.contains('x'), "the URL belongs to the Providers tab, not this row: {drawn:?}");

        f.tab = ConfigTab::Providers;
        f.field = 1;
        let (_, hits, term) = rendered(&f, UiLanguage::Es, 60, 20);
        let row = hits.iter().find(|h| h.key == KeyInput::Enter && h.area.y > 3).expect("a row hit");
        let drawn = text_at(&term, row.area);
        assert!(drawn.chars().all(|c| c == 'x' || c == ' '), "the value fills its column and stops: {drawn:?}");
        assert!(
            row.area.right() <= 60,
            "the value's column ends at the frame: {:?}",
            row.area
        );
    }

    #[test]
    fn every_tab_renders_without_panicking_near_the_layout_minimum() {
        for (f, _) in tabs() {
            for (w, h) in [(1, 1), (20, 5), (40, 12), (60, 20), (80, 24)] {
                let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                // The hits belong to the frame's own tests above; here only the drawing matters.
                term.draw(|frame| {
                    draw_form(frame, &f, strings(UiLanguage::Es), 0);
                })
                .unwrap_or_else(|e| panic!("{:?} failed to draw on {w}x{h}: {e}", f.tab));
            }
        }
        // The screen has nothing to draw on a frame this small, and says so by returning no hits
        // rather than by reporting a rectangle nobody can see. The sentinel is what proves the
        // closure ran at all: a draw that never happened would have left it in place.
        let mut term = Terminal::new(TestBackend::new(1, 1)).unwrap();
        let mut hits = vec![Hit { area: Rect::ZERO, key: KeyInput::Enter }];
        term.draw(|frame| hits = draw_form(frame, &providers(), strings(UiLanguage::Es), 0)).unwrap();
        assert!(hits.is_empty(), "a 1x1 frame has no clickable area, and it reports none: {hits:?}");
    }
}
