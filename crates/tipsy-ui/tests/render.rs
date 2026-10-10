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
    let mut term = Terminal::new(TestBackend::new(110, 30)).unwrap();
    let mut hits = Vec::new();
    term.draw(|f| hits = draw(f, s, &clock(), "Compae · el asistente de garrafón")).unwrap();
    let text: String = term.backend().buffer().content.iter().map(|c| c.symbol()).collect();
    (text, hits, term)
}

fn text_at(term: &Terminal<TestBackend>, r: ratatui::layout::Rect) -> String {
    (r.x..r.right()).map(|x| term.backend().buffer()[(x, r.y)].symbol().to_string()).collect()
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
