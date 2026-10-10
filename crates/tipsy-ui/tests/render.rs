use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tipsy_protocol::*;
use tipsy_ui::*;

fn state(lang: UiLanguage) -> UiState {
    let mut s = UiState::new();
    s.apply(&Event::Welcome {
        session_id: 1,
        persona: PersonaView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
        ui_language: lang,
        listening_mode: ListeningMode::Push,
        web_bundle: false,
    });
    s
}

fn clock() -> ClockView {
    ClockView { time: "10:42:07".into(), weekday: 4, day: 10, month: 9 }
}

fn render(s: &UiState) -> (String, Vec<Hit>, Terminal<TestBackend>) {
    // Wide enough for a whole reply line at a 30% left column: the log wraps, and the assertions
    // below read the text of one line.
    let mut term = Terminal::new(TestBackend::new(150, 40)).unwrap();
    let mut hits = Vec::new();
    term.draw(|f| hits = draw(f, s, &clock(), "Compae · el asistente de garrafón")).unwrap();
    let text: String = term.backend().buffer().content.iter().map(|c| c.symbol()).collect();
    (text, hits, term)
}

fn text_at(term: &Terminal<TestBackend>, r: ratatui::layout::Rect) -> String {
    (r.x..r.right()).map(|x| term.backend().buffer()[(x, r.y)].symbol().to_string()).collect()
}

/// The inner rect of the panel whose title is `title`, read off the drawn buffer: a panel is a
/// rounded block, so the title sits on the row of its top border, between its `╭` and its `╮`.
fn panel_inner(term: &Terminal<TestBackend>, title: &str) -> ratatui::layout::Rect {
    let buffer = term.backend().buffer();
    let area = buffer.area;
    let text_row = |y: u16| -> String {
        (area.left()..area.right()).map(|x| buffer[(x, y)].symbol().to_string()).collect()
    };

    let top = (area.top()..area.bottom()).find(|&y| text_row(y).contains(title)).expect("the panel title");
    let title_x = {
        let line = text_row(top);
        area.left() + line[..line.find(title).unwrap()].chars().count() as u16
    };
    let left = (area.left()..title_x).rev().find(|&x| buffer[(x, top)].symbol() == "╭").expect("left border");
    let right = (title_x..area.right()).find(|&x| buffer[(x, top)].symbol() == "╮").expect("right border");
    let bottom = (top + 1..area.bottom()).find(|&y| buffer[(left, y)].symbol() == "╰").expect("bottom border");
    ratatui::layout::Rect::new(left + 1, top + 1, right - left - 1, bottom - top - 1)
}

#[test]
fn spanish_hud_shows_every_panel() {
    let (text, _, _) = render(&state(UiLanguage::Es));
    for needle in ["COMPAE · EL ASISTENTE DE GARRAFÓN", "HORA LOCAL", "10:42:07", "viernes, 10 de octubre", "NÚCLEO", "VOZ", "SISTEMA", "CHAT", "RED", "PREGUNTA", "Borrar"] {
        assert!(text.contains(needle), "missing {needle:?}");
    }
}

#[test]
fn english_hud_uses_english_strings() {
    let (text, _, _) = render(&state(UiLanguage::En));
    for needle in ["LOCAL TIME", "Friday, October 10", "CORE", "VOICE", "SYSTEM", "NETWORK", "ASK", "Clear"] {
        assert!(text.contains(needle), "missing {needle:?}");
    }
}

#[test]
fn chat_log_shows_user_and_assistant_lines() {
    let mut s = state(UiLanguage::Es);
    for c in "¿y el ticket?".chars() {
        s.on_key(KeyInput::Char(c));
    }
    s.on_key(KeyInput::Enter);
    s.apply(&Event::Reply { text: "Ozú, quillo, eso es de backend.".into() });
    s.apply(&Event::Done { first_audio_ms: Some(1500), total_ms: 4000 });
    let (text, _, _) = render(&s);
    assert!(text.contains("tú ¿y el ticket?"));
    assert!(text.contains("Ozú, quillo, eso es de backend."));
    assert!(text.contains("1.5s"));
}

#[test]
fn input_box_shows_typed_text_or_placeholder() {
    let mut s = state(UiLanguage::Es);
    let (text, _, _) = render(&s);
    assert!(text.contains("Pregúntale algo"));
    for c in "hola".chars() {
        s.on_key(KeyInput::Char(c));
    }
    let (text, _, _) = render(&s);
    assert!(text.contains("❯ hola"));
}

#[test]
fn network_tab_shows_chart_legend() {
    let mut s = state(UiLanguage::Es);
    s.apply(&Event::Stats { cpu: 10.0, ram: 20.0, net_in: 2048.0, net_out: 10.0 });
    s.on_key(KeyInput::Right);
    let (text, _, _) = render(&s);
    assert!(text.contains("IN ") && text.contains("2.0 KB/s"));
}

#[test]
fn listening_shows_red_indicator_text() {
    let mut s = state(UiLanguage::Es);
    s.on_key(KeyInput::Tab);
    let (text, _, _) = render(&s);
    assert!(text.contains("Escuchando"));
}

#[test]
fn clickable_areas_sit_on_their_labels() {
    let (_, hits, term) = render(&state(UiLanguage::Es));
    let labels: Vec<(String, KeyInput)> = hits.iter().map(|h| (text_at(&term, h.area), h.key)).collect();
    assert_eq!(labels, vec![
        (" CHAT ".to_string(), KeyInput::F(7)),
        (" RED ".to_string(), KeyInput::F(8)),
        (" ✗ Borrar ".to_string(), KeyInput::F(4)),
    ]);
}

#[test]
fn rings_fit_inside_their_panel_when_it_is_not_wide_enough_for_them() {
    // Taller than the rings are wide: the old window was fixed vertically and only compensated the
    // shape of a terminal cell across, so a panel this narrow drew the rings far larger than it.
    let s = state(UiLanguage::Es);
    for (w, h) in [(110, 60), (100, 50)] {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| {
            draw(f, &s, &clock(), "x");
        })
        .unwrap();

        // The outermost ring is a full circle, so the middle row of its panel always crosses it and
        // the horizontal extremes do not move with the clock.
        let inner = panel_inner(&term, "NÚCLEO");
        let y = inner.y + inner.height / 2;
        let buffer = term.backend().buffer();
        let drawn: Vec<u16> = (inner.x..inner.right()).filter(|&x| buffer[(x, y)].symbol() != " ").collect();
        let first = *drawn.first().expect("the outer ring crosses the middle row of its panel");
        let last = *drawn.last().unwrap();

        assert!(first > inner.x, "the ring touches the left border of its panel at {w}x{h}");
        assert!(last + 1 < inner.right(), "the ring touches the right border of its panel at {w}x{h}");
        // A window scaled until the drawing fits leaves the middle row almost as wide as the panel.
        // The old one put the horizontal extremes of the ring outside the window, and ratatui drops
        // the points that fall outside it, so only the small centre circle was left visible.
        let span = last - first + 1;
        assert!(span >= inner.width * 3 / 4, "the ring must fill its panel at {w}x{h}: {span} of {} columns", inner.width);
    }
}

#[test]
fn tiny_terminals_do_not_panic() {
    let s = state(UiLanguage::Es);
    for (w, h) in [(1, 1), (20, 5), (40, 12)] {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| {
            draw(f, &s, &clock(), "x");
        })
        .unwrap();
    }
}
