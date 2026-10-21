use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tipsy_protocol::*;
use tipsy_ui::*;

fn state(lang: UiLanguage) -> UiState {
    let mut s = UiState::new();
    // `web_bundle: true` throughout this file: these tests are about the HUD's layout and copy, and a
    // binary that reports no bundle raises a notice that is one line of it — a subject with its own
    // test in `state.rs` (`a_binary_without_the_bundle_says_so_in_both_languages`).
    s.apply(&Event::Welcome {
        session_id: 1,
        profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
        ui_language: lang,
        listening_mode: ListeningMode::Push,
        web_bundle: true,
        login: None,
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
    let text_row =
        |y: u16| -> String { (area.left()..area.right()).map(|x| buffer[(x, y)].symbol().to_string()).collect() };

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
    for needle in [
        "COMPAE · EL ASISTENTE DE GARRAFÓN",
        "HORA LOCAL",
        "10:42:07",
        "viernes, 10 de octubre",
        "NÚCLEO",
        "VOZ",
        "SISTEMA",
        "CHAT",
        "RED",
        "PREGUNTA",
        "Borrar",
    ] {
        assert!(text.contains(needle), "missing {needle:?}");
    }
}

#[test]
fn compact_view_draws_the_core_without_the_hud() {
    let mut term = Terminal::new(TestBackend::new(36, 20)).unwrap();
    let mut hits = Vec::new();
    term.draw(|frame| hits = draw_compact(frame, &state(UiLanguage::Es))).unwrap();
    let text: String = term.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
    assert!(!text.contains("CHAT"));
    assert!(!text.contains("NÚCLEO"));
    assert!(text.contains("VOZ"), "the compact scope is rendered beneath the core");
    assert!(text.chars().any(|character| character != ' '));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].key, KeyInput::F(3), "clicking the core toggles the existing mic control");
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
    // The header's own two controls come first and are checked in
    // `the_gears_area_sits_on_its_label_and_opens_the_screen`; the HUD's three are what this test
    // is about, and they are read in the order `draw_left` reports them.
    let labels: Vec<(String, KeyInput)> = hits
        .iter()
        .filter(|h| [KeyInput::F(7), KeyInput::F(8), KeyInput::F(4)].contains(&h.key))
        .map(|h| (text_at(&term, h.area), h.key))
        .collect();
    assert_eq!(
        labels,
        vec![
            (" CHAT ".to_string(), KeyInput::F(7)),
            (" RED ".to_string(), KeyInput::F(8)),
            (" ✗ Borrar ".to_string(), KeyInput::F(4)),
        ]
    );
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
        assert!(
            span >= inner.width * 3 / 4,
            "the ring must fill its panel at {w}x{h}: {span} of {} columns",
            inner.width
        );
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

#[test]
fn f9_opens_the_configuration_screen_and_esc_closes_it() {
    let mut s = state(UiLanguage::Es);
    s.on_key(KeyInput::F(9));
    let (text, _, _) = render(&s);
    assert!(text.contains("CONFIGURACIÓN"), "F9 opens the screen:\n{text}");
    for needle in ["Proveedores", "Escucha", "Perfiles", "Servidor", "Conector", "Guardar"] {
        assert!(text.contains(needle), "the screen is missing {needle:?}:\n{text}");
    }
    // Esc closes the screen instead of quitting the HUD.
    let actions = s.on_key(KeyInput::Esc);
    assert_eq!(actions, Vec::new(), "Esc belongs to the screen while it is open");
    let (text, _, _) = render(&s);
    assert!(text.contains("NÚCLEO"), "the HUD is back:\n{text}");
    assert!(text.contains("PREGUNTA"));
    assert!(!text.contains("Proveedores"));

    // And F9 closes it too, so the one key §9 names is not a one-way door.
    s.on_key(KeyInput::F(9));
    assert!(render(&s).0.contains("CONFIGURACIÓN"));
    s.on_key(KeyInput::F(9));
    assert!(!render(&s).0.contains("CONFIGURACIÓN"));
}

#[test]
fn the_screen_swallows_the_hud_keys_while_it_is_open() {
    let mut s = state(UiLanguage::Es);
    s.on_key(KeyInput::F(9));
    assert_eq!(s.on_key(KeyInput::Tab), Vec::new(), "Tab must not open the microphone behind the screen");
    assert!(!s.mic_open);
    for c in "hola".chars() {
        s.on_key(KeyInput::Char(c));
    }
    assert!(s.input.is_empty(), "typing in the screen does not fill the question box");
    assert_eq!(s.on_key(KeyInput::Enter), Vec::new(), "and Enter does not send a question");
    assert!(s.log.is_empty());
    // The microphone itself never moved, so closing the screen leaves the HUD as it was.
    s.on_key(KeyInput::Esc);
    assert!(!s.mic_open);
}

#[test]
fn the_header_shows_the_gear_and_the_listening_mode() {
    let mut s = state(UiLanguage::Es);
    let (text, hits, term) = render(&s);
    assert!(text.contains("⚙"), "the gear button is in the header:\n{text}");
    assert_eq!(text_at(&term, hits[0].area), " push ", "the mode is named in its own area");

    s.apply(&Event::Welcome {
        session_id: 1,
        profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
        ui_language: UiLanguage::Es,
        listening_mode: ListeningMode::Auto,
        web_bundle: true,
        login: None,
    });
    let (_, hits, term) = render(&s);
    assert_eq!(text_at(&term, hits[0].area), " auto ", "the configured mode, not a fixed word");

    s.apply(&Event::Welcome {
        session_id: 1,
        profile: ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() },
        ui_language: UiLanguage::En,
        listening_mode: ListeningMode::HandsFree,
        web_bundle: true,
        login: None,
    });
    let (_, hits, term) = render(&s);
    assert_eq!(text_at(&term, hits[0].area), " hands free ", "the English table's spelling");

    // … and the microphone is the colour, not the word: the mode stays on screen while it is open,
    // so an open mic cannot overwrite it.
    s.on_key(KeyInput::Tab);
    let (_, hits, term) = render(&s);
    assert_eq!(text_at(&term, hits[0].area), " hands free ", "the mode survives an open microphone");
    let area = hits[0].area;
    assert_eq!(term.backend().buffer()[(area.x + 1, area.y)].style().fg, Some(ratatui::style::Color::Red));
}

#[test]
fn the_gears_area_sits_on_its_label_and_opens_the_screen() {
    // The screen closed: the two header controls are the first two hits, before the HUD's three.
    let mut s = state(UiLanguage::Es);
    let (_, hits, term) = render(&s);
    assert_eq!(hits[1].key, KeyInput::F(9), "the gear answers F9");
    assert_eq!(text_at(&term, hits[1].area), " ⚙ ");
    assert_eq!(hits[0].key, KeyInput::F(9), "and so does the mode indicator");
    assert_eq!(text_at(&term, hits[0].area), " push ", "the indicator's area is its own word");
    assert_eq!(hits[2].key, KeyInput::F(7), "the HUD's own three hits follow, unchanged");

    // Clicking it is the same gesture as pressing it: the key the hit carries opens the screen.
    let (_, hits, _) = render(&s);
    s.on_key(hits[1].key);
    assert!(render(&s).0.contains("CONFIGURACIÓN"), "a click on the gear opens the screen");
    // And the two header controls are still where they were, so the screen can be closed the same
    // way it was opened.
    let (_, hits, term) = render(&s);
    assert_eq!(text_at(&term, hits[1].area), " ⚙ ");
    s.on_key(hits[1].key);
    assert!(!render(&s).0.contains("CONFIGURACIÓN"));
}

#[test]
fn the_screen_survives_a_terminal_too_small_for_it() {
    let mut s = state(UiLanguage::Es);
    s.on_key(KeyInput::F(9));
    for (w, h) in [(1, 1), (20, 5), (40, 12), (60, 20)] {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| {
            draw(f, &s, &clock(), "x");
        })
        .unwrap();
    }
}

#[test]
fn the_header_shows_the_live_indicator_instead_of_anything_else() {
    let mut s = state(UiLanguage::Es);
    s.apply(&Event::Live { active: true });
    // A turn in flight is not a reason to hide it: the call is billing while it lasts, so the
    // spinner that would otherwise be there is exactly what must not cover the indicator.
    s.apply(&Event::Heard { text: "¿y el ticket?".into() });
    s.apply(&Event::ReplyDelta { text: "Aquí andamos".into() });
    s.apply(&Event::Status { state: AssistantState::Thinking, detail: None });
    assert!(s.busy(), "the test is only worth anything while the spinner would be drawn");
    let (text, _, _) = render(&s);
    assert!(text.contains("EN VIVO"), "missing the live indicator");
    assert!(!text.contains("Pensando"), "a spinner may not sit in front of it");
    assert!(!text.contains("Listo"), "the indicator is the header while a call lasts");
}

#[test]
fn english_header_uses_the_english_live_indicator() {
    let mut s = state(UiLanguage::En);
    s.apply(&Event::Live { active: true });
    let (text, _, _) = render(&s);
    assert!(text.contains("LIVE"));
    assert!(!text.contains("EN VIVO"));
    assert!(!text.contains("Ready"));
}

#[test]
fn the_live_indicator_disappears_when_the_call_ends() {
    let mut s = state(UiLanguage::Es);
    s.apply(&Event::Live { active: true });
    s.apply(&Event::Live { active: false });
    let (text, _, _) = render(&s);
    assert!(!text.contains("EN VIVO"), "the indicator is not left behind by a call that ended");
    assert!(text.contains("Listo"), "the header is the quiet one again");
}

// ---- the login screen (spec 2026-10-11-login-screen-design.md, §6–§8) ----

/// The screen up, with the capture in whatever state the test asks for: `open` is the state the
/// client reconciles the microphone to, and `mic_open` is what `● rec` reads.
fn login(voice: bool, open: bool, mic_open: bool) -> UiState {
    let mut s = state(UiLanguage::Es);
    s.show_login(voice);
    s.mic_open = mic_open;
    if open {
        s.login.as_mut().unwrap().open = true;
    }
    s
}

/// One frame of the login screen, and the text drawn on each of its rows: the widget is centred, so
/// reading its rows by their content is more honest than guessing its coordinates.
fn login_frame(s: &UiState, w: u16, h: u16) -> (String, Vec<Hit>, Terminal<TestBackend>) {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut hits = Vec::new();
    term.draw(|f| hits = draw(f, s, &clock(), "Compae · el asistente de garrafón")).unwrap();
    let text: String = term.backend().buffer().content.iter().map(|c| c.symbol()).collect();
    (text, hits, term)
}

/// The row of the frame whose text contains `needle`.
fn row_with(term: &Terminal<TestBackend>, needle: &str) -> String {
    let buffer = term.backend().buffer();
    (0..buffer.area.height)
        .map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .find(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("no row holds {needle:?}:\n{}", frame_text(term)))
}

fn frame_text(term: &Terminal<TestBackend>) -> String {
    let buffer = term.backend().buffer();
    (0..buffer.area.height)
        .map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The login widget is not a full-screen form (spec L5): the HUD's own panels are drawn and then
/// veiled, and only the widget's rectangle is crisp — the core, the one status line and the input box,
/// with the app's own name and the keys on the borders.
#[test]
fn the_login_widget_draws_the_core_the_status_line_and_the_input() {
    let mut s = login(true, true, true);
    for c in "huesped del core".chars() {
        s.on_key(KeyInput::Char(c));
    }
    let (text, _, term) = login_frame(&s, 150, 40);

    // The title on the top border is the *application's* name, never the profile's: the profile speaks
    // from inside, and its badge is behind the veil.
    assert!(text.contains("◉ tipsy-assistant"), "the widget's top border:\n{text}");
    // The widget's border is a rounded block, like every other panel of the HUD.
    assert!(text.contains("╭") && text.contains("╮") && text.contains("╰") && text.contains("╯"), "a bordered widget");
    // The core is the HUD's own drawing, without the HUD's panel: `◉` in the middle of the widget and
    // no "NÚCLEO" title on it.
    assert!(text.contains('◉') && text.contains("tipsy-assistant"));
    assert!(!text.contains("NÚCLEO"), "the widget's border is the core's frame:\n{text}");
    // One status line: the prompt, because nothing has been said yet. It is cut to the widget's own
    // width, so the assertion reads the head of the sentence rather than the whole of it.
    assert!(text.contains("¿Y tú quién eres?"), "the status line:\n{text}");
    assert!(!text.contains("cualquiera"), "the status line is cut to the widget, never wrapped:\n{text}");
    // And the input row, with what was typed.
    assert!(text.contains("❯ huesped del core"), "the input row:\n{text}");
    // The keys ride on the bottom border.
    assert!(text.contains("Tab micro · Enter entrar · esc salir"), "the bottom border:\n{text}");
    let _ = term;
}

/// The badge is the only thing that says the microphone is open, and it appears exactly while the
/// capture runs (spec §6: the v1 visible-indicator rule kept without a header).
#[test]
fn the_recording_badge_appears_exactly_while_the_microphone_is_open() {
    let open = login(true, true, true);
    let (text, _, _) = login_frame(&open, 150, 40);
    assert!(text.contains(strings(UiLanguage::Es).login_rec), "an open capture must say so:\n{text}");

    let closed = login(true, false, false);
    let (text, _, _) = login_frame(&closed, 150, 40);
    assert!(!text.contains(strings(UiLanguage::Es).login_rec), "a shut door is not recording:\n{text}");

    // And it is red: the colour is the message, not the word.
    let (_, _, term) = login_frame(&open, 150, 40);
    let rec = row_with(&term, strings(UiLanguage::Es).login_rec);
    let buffer = term.backend().buffer();
    let y = (0..buffer.area.height).find(|&y| row_text(&term, y) == rec).unwrap();
    let x = (0..buffer.area.width).find(|&x| buffer[(x, y)].symbol() == "●").expect("the recording dot");
    assert_eq!(buffer[(x, y)].style().fg, Some(ratatui::style::Color::Red), "● rec is red");
}

fn row_text(term: &Terminal<TestBackend>, y: u16) -> String {
    let buffer = term.backend().buffer();
    (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect()
}

/// Review finding 3, on the widget: `● rec` is the *login's* capture and not the HUD's `Tab` switch.
/// The switch belongs to a microphone the veil is hiding, and the door may well find it in either
/// position — a lit badge over a screen that is not recording would be the one lie §6 cannot tell.
#[test]
fn the_recording_badge_follows_the_screens_own_capture() {
    // The HUD's switch is on and the screen's capture is not: nothing is recording.
    let mut hud_only = state(UiLanguage::Es);
    hud_only.mic_open = true;
    hud_only.show_login(true);
    let (text, _, _) = login_frame(&hud_only, 150, 40);
    assert!(!text.contains(strings(UiLanguage::Es).login_rec), "the HUD's switch is not this screen's:\n{text}");

    // The screen's capture is open while the HUD's switch is off: it is recording, and says so.
    let mut login_only = state(UiLanguage::Es);
    login_only.show_login(true);
    login_only.login.as_mut().unwrap().open = true;
    assert!(!login_only.mic_open, "the client no longer writes the HUD's switch for the login");
    let (text, _, _) = login_frame(&login_only, 150, 40);
    assert!(text.contains(strings(UiLanguage::Es).login_rec), "the screen's own capture is lit:\n{text}");
}

/// `Tab` is consent and the guard only stops *automatic* listening: the third refusal in a row is
/// where the bottom border swaps to the escape hatch (spec §6, L8).
#[test]
fn the_bottom_border_swaps_to_the_escape_hint_after_three_refusals() {
    let mut s = login(true, true, true);
    assert_eq!(s.login.as_ref().unwrap().refusals, 0);
    for _ in 0..2 {
        s.apply(&Event::LoginVerdict { ok: false, name: None });
    }
    let (text, _, _) = login_frame(&s, 150, 40);
    assert!(text.contains("Tab micro · Enter entrar · esc salir"), "attempts 1 and 2 keep the keys:\n{text}");
    assert!(!text.contains("config.toml"), "and do not point at the configuration yet");

    s.apply(&Event::LoginVerdict { ok: false, name: None });
    let (text, _, _) = login_frame(&s, 150, 40);
    assert!(text.contains("Tab micro · Enter entrar · ¿olvidado? [login]"), "the third says how to get out:\n{text}");
    assert!(!text.contains("esc salir"), "and the hint replaces the keys rather than joining them");
}

/// Review Focus 5: the heard line is a person's own text. Cutting it to the widget's width must land
/// on a character boundary — a naive `&text[..width]` panics on `é`, which is a letter the profiles
/// this project ships write.
#[test]
fn a_two_hundred_character_heard_line_is_cut_on_a_character_boundary() {
    let mut s = login(true, false, false);
    let long: String = "é".repeat(LOGIN_INPUT_MAX);
    assert_eq!(long.chars().count(), LOGIN_INPUT_MAX, "the cap's own worst case");
    s.apply(&Event::LoginHeard { text: long.clone() });
    s.apply(&Event::LoginVerdict { ok: false, name: None });

    for (w, h) in [(150, 40), (60, 20), (40, 12), (21, 5), (20, 4)] {
        // What this pins is that the frame is drawn at all: a cut that landed inside `é` would have
        // panicked in the buffer write long before the assertion below.
        let (_, _, _) = login_frame(&s, w, h);
    }
    // The heard line is on screen, cut to the widget: the status line of a full frame is the refusal
    // that replaced it, and the refusal's own head is what the row carries.
    let (text, _, term) = login_frame(&s, 150, 40);
    assert!(text.contains("Anda, que te peinen para el lado."), "the status line:\n{text}");
    // The 150-column frame is wide enough for the heard line to survive the *widget*, and the widget
    // is 48 wide: the whole of a 200-character answer cannot be on it, so the row stops at its edge.
    let rows = (0..term.backend().buffer().area.height)
        .map(|y| row_text(&term, y))
        .filter(|row| row.contains('é'))
        .collect::<Vec<_>>();
    assert!(rows.is_empty(), "the refusal replaced the heard line: {rows:?}");

    // And the heard line itself, with no verdict behind it: its head is drawn, cut and never mangled.
    let mut heard = login(true, false, false);
    heard.apply(&Event::LoginHeard { text: long.clone() });
    let (text, _, term) = login_frame(&heard, 150, 40);
    assert!(text.contains("Te he oído: «"), "the heard wording is on screen:\n{text}");
    assert!(text.contains('é'), "and the answer itself:\n{text}");
    assert!(!text.contains('\u{FFFD}'), "and never a replacement character from a cut inside one");
    // The widget is 44 columns inside its border, so the answer stops there: the whole of a
    // 200-character line cannot be on it, and the answer's own `é`s can be counted on the one row.
    // The count matters less than the two facts around it: every `é` on that row is a whole one (a
    // cut inside one would have written a replacement character, or panicked), and they are fewer
    // than the 200 that were heard.
    let row = row_with(&term, "Te he oído");
    assert!(row.contains("«"), "the guillemets are the spec's own");
    let drawn = row.matches('é').count();
    assert!(drawn > 0 && drawn < LOGIN_INPUT_MAX, "{drawn} of {LOGIN_INPUT_MAX}");

    // The same for a typed answer: the box shows what is in it, cut and never mangled.
    let mut typed = login(true, false, false);
    for c in long.chars() {
        typed.on_key(KeyInput::Char(c));
    }
    let (text, _, _) = login_frame(&typed, 60, 20);
    assert!(text.contains('❯'), "the input row is drawn:\n{text}");
}

/// Review finding 2, on the screen itself: while the answer is with the transcriber the row says so,
/// because the alternative is a refused screen that looks frozen — and a user who thinks the machine
/// did not hear them says the answer again, which cancels the transcription in flight (spec §4.1).
#[test]
fn a_login_answer_being_transcribed_says_so_on_the_status_line() {
    let mut s = login(true, true, true);
    // A refusal that has been read: the capture is still open, so the row is the prompt and not a
    // verdict the user has already seen.
    s.apply(&Event::LoginVerdict { ok: false, name: None });
    assert!(login_frame(&s, 150, 40).0.contains("¿Y tú quién eres?"), "the row is ready again");

    s.apply(&Event::Status { state: AssistantState::Transcribing, detail: None });
    let (text, _, _) = login_frame(&s, 150, 40);
    assert!(text.contains(strings(UiLanguage::Es).transcribing), "the wait has a face:\n{text}");
    assert!(!text.contains("Anda, que te peinen"), "and is not the last refusal over again:\n{text}");

    // What was heard is the next face of the same wait, and the verdict ends it.
    s.apply(&Event::LoginHeard { text: "soy huesped".into() });
    let (text, _, _) = login_frame(&s, 150, 40);
    assert!(text.contains("Te he oído"), "the heard line:\n{text}");
    assert!(!text.contains(strings(UiLanguage::Es).transcribing), "the wait is over:\n{text}");

    // The third refusal in a row is the one that shuts the capture, and there the row keeps it: there
    // is no next attempt for it to be stale in, and it is read beside the escape hint.
    s.apply(&Event::LoginVerdict { ok: false, name: None });
    s.apply(&Event::LoginVerdict { ok: false, name: None });
    let (text, _, _) = login_frame(&s, 150, 40);
    assert!(text.contains("Anda, que te peinen"), "the last refusal is what the row keeps:\n{text}");
    // The escape hint is on the bottom border, cut to the widget's own width — the third-refusal test
    // above is where its whole text is asserted, on a frame wide enough to hold it.
    assert!(text.contains("¿olvidado?"), "beside the escape hint:\n{text}");
}

/// A typed answer is judged rather than recorded, so the row may never claim it is being transcribed:
/// the server says nothing about a typed answer, and the screen must not invent the wait.
#[test]
fn a_typed_answer_is_not_dressed_up_as_a_transcription() {
    let mut s = login(true, true, true);
    for c in "huesped".chars() {
        s.on_key(KeyInput::Char(c));
    }
    s.on_key(KeyInput::Enter);
    let (text, _, _) = login_frame(&s, 150, 40);
    assert!(!text.contains(strings(UiLanguage::Es).transcribing), "a typed answer is not a recording:\n{text}");
}

/// Only what is drawn reports a hit (spec §6): the title answers `Tab`, the input row answers `Enter`,
/// and everything behind the veil reports nothing at all — a click on the dashboard may not open the
/// configuration screen, clear the conversation or replay a reply while the door is shut.
#[test]
fn only_the_title_and_the_input_row_report_a_hit() {
    let mut s = login(true, false, false);
    s.log.push(tipsy_ui::LogEntry {
        who: tipsy_ui::Who::Assistant,
        text: "una respuesta que estaba detrás".into(),
        first_audio_ms: Some(1200),
        error: None,
        pending: false,
    });
    let (text, hits, term) = login_frame(&s, 150, 40);
    assert_eq!(hits.len(), 2, "the widget has exactly two controls: {hits:?}");
    assert_eq!(hits[0].key, KeyInput::Tab, "the title is the microphone");
    assert_eq!(hits[1].key, KeyInput::Enter, "the input row is the door");
    assert!(hits.iter().all(|h| h.action.is_none()), "a click is the keystroke its control means");

    let title = text_at(&term, hits[0].area);
    assert!(title.contains("tipsy-assistant"), "the title's area sits on the title: {title:?}");
    // The input's area is the row *inside* the border — the border is the widget's, and the clickable
    // row is what the border frames.
    let input = text_at(&term, hits[1].area);
    assert!(input.starts_with('❯'), "the input row's area starts at its own caret: {input:?}");
    assert_eq!(input.chars().count() as u16, hits[1].area.width, "and is exactly as wide as it says");
    assert_eq!(input.trim_end(), "❯", "nothing was typed in this frame");
    // The HUD's own three hits — chat, network, clear — are nowhere in the list, and neither is the
    // header's gear: they were computed behind the veil and thrown away.
    assert!(!hits.iter().any(|h| h.key == KeyInput::F(9)), "the gear is out of focus while the door is shut");
    assert!(!text.contains("Borrar"), "the dashboard's own controls are veiled:\n{text}");

    // Pressing `Tab` is exactly what clicking the title does: both go through the same key.
    assert_eq!(s.on_hit(&hits[0]).len(), 1, "the title's key is the microphone key");
    assert!(s.login.as_ref().unwrap().open);
}

/// The cursor belongs to the login's own box, not to the question box behind the veil: it follows
/// what the screen has received, and it is on the screen at all — the HUD sets one for its own input
/// row every frame, and a screen that left it there would put the caret on a cell nobody can read.
#[test]
fn the_cursor_follows_the_login_box() {
    let mut s = login(true, false, false);
    let mut term = Terminal::new(TestBackend::new(150, 40)).unwrap();
    term.draw(|f| {
        draw(f, &s, &clock(), "x");
    })
    .unwrap();
    let input = term.backend().buffer();
    let row = (0..input.area.height)
        .find(|&y| (0..input.area.width).any(|x| input[(x, y)].symbol() == "❯"))
        .expect("the widget draws its input row");
    let caret = term.get_cursor_position().expect("the frame must place the cursor");
    assert_eq!(caret.y, row, "the cursor is on the input row");
    let x0 = caret.x;

    for c in "huesped".chars() {
        s.on_key(KeyInput::Char(c));
    }
    term.draw(|f| {
        draw(f, &s, &clock(), "x");
    })
    .unwrap();
    let moved = term.get_cursor_position().expect("the frame must place the cursor");
    assert_eq!(moved.y, row);
    assert_eq!(moved.x, x0 + 6, "the caret follows the characters the box has taken");
}

/// Spec §6's degradation: a frame too small for a bordered widget gets the bare input row on its last
/// line, with no hit at all — the login is never the thing that makes a tiny terminal unusable.
#[test]
fn a_tiny_frame_draws_the_bare_input_row_and_reports_no_hit() {
    let mut s = login(true, false, false);
    for c in "and".chars() {
        s.on_key(KeyInput::Char(c));
    }
    let (text, hits, term) = login_frame(&s, 12, 2);
    assert!(hits.is_empty(), "nothing is clickable on a frame this small: {hits:?}");
    assert!(!text.contains('┌') && !text.contains('│'), "no border:\n{text}");
    let last = row_text(&term, 1);
    assert!(last.contains("❯ and"), "the bare input row is on the last line: {last:?}");
}

/// The ten strings of spec §8, reproduced verbatim in both tables, «guillemets» included: the copy is
/// the record of what the user approved, and a reworded line is a change to the product's voice.
#[test]
fn the_login_copy_is_the_specs_own_words_in_both_languages() {
    let es = strings(UiLanguage::Es);
    let en = strings(UiLanguage::En);
    assert_eq!(es.login_prompt, "¿Y tú quién eres? Dímelo, que aquí no entra cualquiera.");
    assert_eq!(en.login_prompt, "So who are you, then? Tell me — this door isn't for just anyone.");
    assert_eq!(es.login_heard, "Te he oído: «{text}»");
    assert_eq!(en.login_heard, "I heard: \"{text}\"");
    assert_eq!(es.login_refusal, "Anda, que te peinen para el lado.");
    assert_eq!(en.login_refusal, "Off you go, and get your hair combed somewhere else.");
    assert_eq!(es.login_pass, "Pasa, {name}.");
    assert_eq!(en.login_pass, "Come in, {name}.");
    assert_eq!(es.login_pass_anon, "Pasa.");
    assert_eq!(en.login_pass_anon, "Come in.");
    assert_eq!(es.login_nothing_heard, "No te he oído. Repite, si eso.");
    assert_eq!(en.login_nothing_heard, "I heard nothing. Try again, if you like.");
    assert_eq!(es.login_no_mic, "Sin micro: escribe, que también vale.");
    assert_eq!(en.login_no_mic, "No mic: type it, that works too.");
    assert_eq!(es.login_rec, "● rec");
    assert_eq!(en.login_rec, "● rec");
    assert_eq!(es.login_keys, "Tab micro · Enter entrar · esc salir");
    assert_eq!(en.login_keys, "Tab mic · Enter in · esc quit");
    assert_eq!(es.login_keys_lost, "Tab micro · Enter entrar · ¿olvidado? [login] en config.toml");
    assert_eq!(en.login_keys_lost, "Tab mic · Enter in · forgotten it? [login] in config.toml");
}
