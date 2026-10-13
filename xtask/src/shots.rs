//! `xtask shots`: draw the HUD's own frames into `docs/images/`, so the README's pictures are what
//! the application really renders instead of a mock-up someone has to keep in step by hand.
//!
//! The frames come from `tipsy_ui::draw` over `TestBackend` — the same call the HUD's own tests make —
//! so a picture is a frame the layout is already known to survive, and no key, microphone, display or
//! network call is involved. The buffer is then written out as SVG: **one absolutely positioned run of
//! text per styled span, each with a `textLength`**, which is what keeps the columns aligned whatever
//! monospace font the reader's browser resolves (the web terminal's own defect, `8c2a719`: a font that
//! lacks the HUD's box-drawing glyphs falls back per glyph and the advance stops matching the cell).
//!
//! The HUD is not the whole product: the browser client and the web terminal are pictures no
//! `TestBackend` can produce. Those are captured from a running session by hand and live beside these.

use std::fmt::Write as _;
use std::path::Path;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};
use tipsy_assistant::config::Config;
use tipsy_assistant::config::form::form_of;
use tipsy_assistant::config::model::ProviderKind;
use tipsy_protocol::{AssistantState, ListeningMode, ProfileView, UiLanguage};
use tipsy_ui::{ClockView, ConfigTab, LogEntry, Tab, UiState, Who, draw};

/// The frame every shot is drawn at, in terminal cells. This is the size the HUD's own render tests
/// use, and a width a real terminal has, so the content wraps where it does on the user's screen.
const WIDTH: u16 = 120;

/// One picture: the file it is written to, the frame it is drawn at, and the state it shows.
///
/// The height is per scene because the HUD fills a terminal and the configuration screen does not:
/// the Providers tab is fifteen rows in a thirty-row body, and a picture with its bottom third empty
/// reads as a screen that failed to draw rather than as a roomy one.
struct Scene {
    name: &'static str,
    height: u16,
    state: UiState,
}

/// One cell, in pixels. The font is a little smaller than the cell because a monospace face's glyphs
/// are taller than they look; `textLength` corrects whatever advance the reader's font would pick.
const CELL_W: f64 = 9.6;
const CELL_H: f64 = 20.0;
const FONT_SIZE: f64 = 16.0;
/// Where the baseline sits inside a cell.
const BASELINE: f64 = 15.4;

/// The sheet the HUD is drawn on, and the ink of a cell that names no colour of its own: what
/// `Color::Reset` means to a terminal with a dark theme.
const BASE_BG: &str = "#0b0f14";
const BASE_FG: &str = "#c9d4e0";

/// The faces the SVG asks for, the same order the web terminal settled on: a font that carries the
/// box-drawing, block and geometric glyphs the HUD draws comes first, and `Consolas` is a candidate
/// rather than the answer.
const FONT_FAMILY: &str = "DejaVu Sans Mono,Cascadia Mono,Consolas,Liberation Mono,Menlo,monospace";

/// Writes one SVG per scene into `docs/images/`.
pub fn run(root: &Path) -> Result<(), String> {
    let dir = root.join("docs").join("images");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for scene in scenes() {
        let buffer = render(&scene.state, scene.height);
        let path = dir.join(format!("{}.svg", scene.name));
        std::fs::write(&path, svg(&buffer)).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

/// Every picture this task knows how to draw, in the order they read. The sixty-line frame with the
/// conversation and the login is the HUD; the configuration shots are as tall as the tab they show,
/// which is what the arithmetic in `draw_form`'s own layout says: three head rows, the rows, one
/// blank, the action line and the help line.
///
/// Two languages on purpose. The README is English and its audience is on GitHub, and the interface
/// is drawn in the *profile's* language — so the default profile's own Spanish is what a first run
/// really shows, and the second picture is the same HUD with an English-speaking profile, which is
/// the whole of the localisation claim in one image.
fn scenes() -> Vec<Scene> {
    vec![
        Scene { name: "hud-chat", height: 36, state: chat() },
        Scene { name: "hud-chat-en", height: 36, state: chat_en() },
        Scene { name: "hud-network", height: 36, state: network() },
        Scene { name: "config-providers", height: 21, state: config_providers() },
        Scene { name: "config-profiles", height: 26, state: config_profiles() },
        Scene { name: "login", height: 36, state: login() },
    ]
}

/// One frame, drawn the way the client draws it and read back off the backend.
fn render(state: &UiState, height: u16) -> Buffer {
    let mut term = Terminal::new(TestBackend::new(WIDTH, height)).expect("a terminal over TestBackend");
    let clock = ClockView { time: "10:42:07".into(), weekday: 4, day: 10, month: 9 };
    // The title is derived here exactly as `client::draw_frame` derives it from the running state —
    // the profile's own name — rather than written out a second time where it could drift.
    let title = state.profile.as_ref().map(|profile| profile.name.clone()).unwrap_or_default();
    term.draw(|frame| {
        draw(frame, state, &clock, &title);
    })
    .expect("a frame is drawn without touching a real terminal");
    term.backend().buffer().clone()
}

/// The state every scene starts from, drawn as `profile`.
fn base(profile: ProfileView) -> UiState {
    let mut state = UiState::new();
    state.lang = language_of(&profile);
    state.profile = Some(profile);
    state.listening_mode = ListeningMode::Push;
    state.status = AssistantState::Idle;
    state.web_bundle = true;
    state.elapsed_ms = 754_000;
    state.cpu = 14.0;
    state.ram = 44.0;
    state.net_in = 214_000.0;
    state.net_out = 31_000.0;
    state.cpu_history = (0..120).map(cpu_sample).collect();
    state.net_history = (0..120).map(net_sample).collect();
    state.voice_envelope = envelope();
    // The head sits a little behind the end of the envelope, so the oscilloscope has a window to look
    // back through and the core's pulse has a level to read.
    state.voice_pos_ms = 160 * 50;
    state
}

/// The interface language the server announces for a profile: Spanish when the profile's own language
/// starts with `es` and English otherwise. It is the rule the README states, applied the way the
/// session applies it — a scene's `UiLanguage` is never written by hand beside its profile.
fn language_of(profile: &ProfileView) -> UiLanguage {
    if profile.language.starts_with("es") { UiLanguage::Es } else { UiLanguage::En }
}

/// The profile the README leads with: the default one, and the only one that is Spanish *and* first.
fn compae() -> ProfileView {
    ProfileView { id: "compae".into(), name: "El Compae".into(), language: "es-ES".into() }
}

/// An English profile, for the second picture: chosen because its register is the one the other
/// profiles are written in, and because it is not the default — the caption says as much.
fn mate() -> ProfileView {
    ProfileView { id: "mate".into(), name: "The Mate".into(), language: "en-AU".into() }
}

/// A chat that has already happened, so the log is a conversation and not a placeholder. The Spanish
/// one is El Compae's and the English one is The Mate's: same question, same answer, each in the
/// voice the profile ships with.
fn chat() -> UiState {
    let mut state = base(compae());
    state.log = vec![
        user("El pipeline de payments peta en el stage de tests. ¿Le echas un ojo?"),
        reply(
            "Ozú, quillo, que ya voy... Mira, el stage revienta porque el `npm test` corre antes del \
             `npm ci`, y el runner se queda sin `vitest`. Ponle el `npm ci` delante y arreglao. \
             Cortito, ¿eh? Que luego me llevas de cañas.",
            1_180,
        ),
        user("¿Y por qué solo falla en la rama de release?"),
        reply(
            "Porque esa usa el runner cacheado, mi arma, y el cache no trae las devDependencies. \
             Le plantas un `npm ci` seco al principio y deja de llorar.",
            940,
        ),
    ];
    state
}

/// The same conversation the other way round: the English profile, and the interface with it.
fn chat_en() -> UiState {
    let mut state = base(mate());
    state.log = vec![
        user("The payments pipeline keeps dying in the test stage. Can you have a look?"),
        reply(
            "Yeah nah, easy fixed, mate. Your `npm test` is running before the `npm ci`, so the runner \
             hasn't got `vitest` on it. Bung the install in first and she'll be right. Five-minute job, \
             that. Righto, are we getting a coffee or what?",
            1_240,
        ),
        user("Why does it only blow up on the release branch?"),
        reply(
            "Because that one's on a cached runner, mate, and the cache hasn't got the \
             devDependencies in it. Dry `npm ci` at the top and it'll pull its head in.",
            980,
        ),
    ];
    state
}

/// The same session with the NETWORK view up: the log's pane holds the traffic chart instead.
fn network() -> UiState {
    let mut state = chat();
    state.tab = Tab::Network;
    state
}

/// The configuration screen, on one of its tabs, with the focus on a row that reads.
fn config_screen(config: Config, tab: ConfigTab, field: usize) -> UiState {
    let mut state = base(compae());
    state.log = chat().log;
    let mut form = form_of(&config);
    form.tab = tab;
    form.field = field;
    state.show_config_form(form);
    state
}

/// The Providers tab as a configuration a user might really write: one *different* connector per
/// capability. The default is OpenRouter three times over, which would not show that `kind` is a
/// choice at all. Every pair here is one the application accepts: Gemini speaks, ElevenLabs talks.
fn config_providers() -> UiState {
    let mut config = Config::default();
    config.providers.chat.kind = ProviderKind::Gemini;
    config.providers.chat.base_url = "https://generativelanguage.googleapis.com".into();
    config.providers.chat.model = "gemini-2.5-flash".into();
    config.providers.chat.key = "google".into();
    config.providers.speech.kind = ProviderKind::Elevenlabs;
    config.providers.speech.base_url = "https://api.elevenlabs.io/v1".into();
    config.providers.speech.model = "eleven_multilingual_v2".into();
    config.providers.speech.key = "elevenlabs".into();
    config_screen(config, ConfigTab::Providers, 1)
}

/// The Profiles tab, where the nine profiles that ship are read and the active one is chosen.
fn config_profiles() -> UiState {
    config_screen(Config::default(), ConfigTab::Profiles, 3)
}

/// The login screen: the HUD is behind it and veiled, the widget is in the middle, and an answer is
/// half typed into its box.
fn login() -> UiState {
    let mut state = base(compae());
    state.log = chat().log;
    state.status = AssistantState::Listening;
    state.mic_open = true;
    state.show_login(true);
    if let Some(login) = state.login.as_mut() {
        login.open = true;
        login.input = "soy Andrés, del equipo del core".into();
    }
    state
}

fn user(text: &str) -> LogEntry {
    LogEntry { who: Who::User, text: text.into(), first_audio_ms: None, error: None, pending: false }
}

/// An assistant entry that was spoken, so the log shows its timing beside the text.
fn reply(text: &str, first_audio_ms: u64) -> LogEntry {
    LogEntry {
        who: Who::Assistant,
        text: text.into(),
        first_audio_ms: Some(first_audio_ms),
        error: None,
        pending: false,
    }
}

/// A CPU trace with the shape a quiet machine has: mostly idle, a few busy stretches, and the jitter
/// a sampler really carries. A smooth curve draws as a hill and reads as a chart, not as a machine.
fn cpu_sample(i: usize) -> f32 {
    let t = i as f32;
    let wave = (t / 5.0).sin() * 0.5 + 0.5;
    let burst = ((t / 19.0).sin() * 0.5 + 0.5).powi(4);
    let jitter = ((t * 7.3).sin() * (t * 3.1).cos()).abs();
    (6.0 + 18.0 * wave * (1.0 + 1.6 * burst) + 9.0 * jitter).min(100.0)
}

/// Two minutes of traffic: a steady trickle of inbound bytes with a reply's worth of outbound.
fn net_sample(i: usize) -> (f64, f64) {
    let t = i as f64;
    let inbound = 140_000.0 + 90_000.0 * (t / 11.0).sin() + 40_000.0 * (t / 3.0).cos();
    let outbound = 12_000.0 + 22_000.0 * ((t / 17.0).sin() * 0.5 + 0.5);
    (inbound.max(0.0), outbound.max(0.0))
}

/// Loudness over time, in `0.0..=1.0`: the HUD's own scale, with the pauses a sentence has.
fn envelope() -> Vec<f32> {
    (0..200)
        .map(|i| {
            let t = i as f32 / 50.0;
            let sentence = ((t * 1.7).sin() * 0.5 + 0.5).powf(0.6);
            let syllables = (t * 12.0).sin() * 0.5 + 0.5;
            (0.12 + 0.78 * sentence * syllables).clamp(0.0, 1.0)
        })
        .collect()
}

/// The frame as SVG: a background rect per run of one colour, then one `<text>` per styled run.
///
/// Runs are what keep the file small — a 150×40 frame is 6,000 cells and a few hundred runs — and the
/// `textLength` on each of them is what keeps the columns where the buffer put them.
fn svg(buffer: &Buffer) -> String {
    let area = buffer.area;
    let (w, h) = (f64::from(area.width) * CELL_W, f64::from(area.height) * CELL_H);
    let cell = |x: u16, y: u16| buffer[(area.left() + x, area.top() + y)].clone();
    let mut out = String::new();
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.0}" height="{h:.0}" viewBox="0 0 {w:.0} {h:.0}" font-family="{FONT_FAMILY}">"#
    );
    let _ = writeln!(out, r#"<rect width="{w:.0}" height="{h:.0}" fill="{BASE_BG}"/>"#);

    for y in 0..area.height {
        let mut x = 0;
        while x < area.width {
            let start = x;
            let bg = cell(x, y).style().bg.unwrap_or(Color::Reset);
            while x < area.width && cell(x, y).style().bg.unwrap_or(Color::Reset) == bg {
                x += 1;
            }
            let fill = hex(bg, BASE_BG);
            if fill != BASE_BG {
                let _ = writeln!(
                    out,
                    r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{CELL_H:.0}" fill="{fill}" shape-rendering="crispEdges"/>"#,
                    f64::from(start) * CELL_W,
                    f64::from(y) * CELL_H,
                    f64::from(x - start) * CELL_W,
                );
            }
        }
    }

    for y in 0..area.height {
        let mut x = 0;
        while x < area.width {
            let first = cell(x, y);
            let style = first.style();
            if first.symbol().trim().is_empty() {
                x += 1;
                continue;
            }
            let start = x;
            let mut text = String::new();
            while x < area.width {
                let next = cell(x, y);
                if next.symbol().trim().is_empty() || next.style() != style {
                    break;
                }
                text.push_str(next.symbol());
                x += 1;
            }
            let _ = writeln!(
                out,
                r#"<text x="{:.1}" y="{:.1}" textLength="{:.1}" lengthAdjust="spacingAndGlyphs" font-size="{FONT_SIZE:.0}" fill="{}"{} xml:space="preserve">{}</text>"#,
                f64::from(start) * CELL_W,
                f64::from(y) * CELL_H + BASELINE,
                f64::from(x - start) * CELL_W,
                hex(style.fg.unwrap_or(Color::Reset), BASE_FG),
                attributes(style),
                escape(&text),
            );
        }
    }
    out.push_str("</svg>\n");
    out
}

/// The two things the HUD ever asks of a font: heavier and slanted.
fn attributes(style: Style) -> &'static str {
    match (style.add_modifier.contains(Modifier::BOLD), style.add_modifier.contains(Modifier::ITALIC)) {
        (true, true) => r#" font-weight="bold" font-style="italic""#,
        (true, false) => r#" font-weight="bold""#,
        (false, true) => r#" font-style="italic""#,
        (false, false) => "",
    }
}

/// A colour as `#rrggbb`, with `default` standing in for `Color::Reset`. The palette is xterm's
/// (VS Code's dark theme, which is what the HUD's own `Rgb` values were chosen against), and the
/// 256-colour cube is the standard one so an indexed colour cannot come out wrong.
fn hex(colour: Color, default: &str) -> String {
    const ANSI: [&str; 16] = [
        "#000000", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5", "#666666", "#f14c4c",
        "#23d18b", "#f5f543", "#3b8eea", "#d670d6", "#29b8db", "#ffffff",
    ];
    let cube = |n: u8| {
        let level = |c: u8| if c == 0 { 0 } else { 55 + c * 40 };
        format!("#{:02x}{:02x}{:02x}", level(n / 36), level(n / 6 % 6), level(n % 6))
    };
    match colour {
        Color::Reset => default.to_string(),
        Color::Black => ANSI[0].to_string(),
        Color::Red => ANSI[1].to_string(),
        Color::Green => ANSI[2].to_string(),
        Color::Yellow => ANSI[3].to_string(),
        Color::Blue => ANSI[4].to_string(),
        Color::Magenta => ANSI[5].to_string(),
        Color::Cyan => ANSI[6].to_string(),
        Color::Gray => ANSI[7].to_string(),
        Color::DarkGray => ANSI[8].to_string(),
        Color::LightRed => ANSI[9].to_string(),
        Color::LightGreen => ANSI[10].to_string(),
        Color::LightYellow => ANSI[11].to_string(),
        Color::LightBlue => ANSI[12].to_string(),
        Color::LightMagenta => ANSI[13].to_string(),
        Color::LightCyan => ANSI[14].to_string(),
        Color::White => ANSI[15].to_string(),
        Color::Indexed(n) if n < 16 => ANSI[n as usize].to_string(),
        Color::Indexed(n) if n < 232 => cube(n - 16),
        Color::Indexed(n) => {
            let v = 8 + (n - 232) * 10;
            format!("#{v:02x}{v:02x}{v:02x}")
        }
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
    }
}

/// Text for an XML element body. The HUD's copy carries `&`, `<` and `>` (`F9`/`Esc` hints, `<the
/// Server tab's token>`), and an unescaped one of them is a file a browser refuses to render.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each scene must be the scene it claims. A frame always renders — a blank one renders too — so
    /// what is checked is the copy and the glyphs a picture of *that* screen has to carry.
    #[test]
    fn every_scene_shows_what_it_is_there_to_show() {
        let expected: [(&str, &[&str]); 6] = [
            ("hud-chat", &["EL COMPAE", "◆", "❯", "Listo, quillo"]),
            ("hud-chat-en", &["THE MATE", "◆", "❯", "Ready", "Yeah nah"]),
            ("hud-network", &["NÚCLEO", "RED", "ahora"]),
            ("config-providers", &["CONFIGURACIÓN", "elevenlabs", "gemini"]),
            ("config-profiles", &["El Compae", "oye compae"]),
            ("login", &["Andrés", "╭", "╮"]),
        ];
        for (scene, (want, words)) in scenes().into_iter().zip(expected) {
            assert_eq!(scene.name, want);
            let buffer = render(&scene.state, scene.height);
            // Row by row, because a phrase that wraps is not one run of cells any more.
            let text: String = (0..buffer.area.height)
                .map(|y| {
                    (0..buffer.area.width)
                        .map(|x| buffer[(buffer.area.left() + x, buffer.area.top() + y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            for word in words {
                assert!(text.contains(word), "{} does not show `{word}`:\n{text}", scene.name);
            }
        }
    }

    /// A configuration shot that reached past the rows it has is a picture with a third of a terminal
    /// of nothing under it. The two tabs are drawn at the height where the last row and the action
    /// line are the last two things on the frame.
    #[test]
    fn a_configuration_shot_is_as_tall_as_its_rows() {
        for scene in scenes().into_iter().filter(|s| s.name.starts_with("config-")) {
            let buffer = render(&scene.state, scene.height);
            let rows: Vec<String> = (0..buffer.area.height)
                .map(|y| {
                    (0..buffer.area.width)
                        .map(|x| buffer[(buffer.area.left() + x, buffer.area.top() + y)].symbol())
                        .collect()
                })
                .collect();
            let last = rows.len() - 1;
            assert!(rows[last].contains("F9 configurar"), "{}: the help line is the last row", scene.name);
            assert!(rows[last - 1].contains("Guardar"), "{}: the action line is the one above it", scene.name);
            assert!(
                rows[last - 2].trim_matches(|c| c == ' ' || c == '│').is_empty(),
                "{}: one blank line, no more",
                scene.name
            );
        }
    }

    /// The SVG is what the reader sees, so what matters is the frame's size, its runs and its
    /// escaping — not that a buffer was produced.
    #[test]
    fn the_svg_is_one_run_per_span_at_the_frames_own_size() {
        let state = chat();
        let buffer = render(&state, 36);
        let out = svg(&buffer);

        assert!(out.starts_with("<svg "), "{out:.120}");
        assert!(out.ends_with("</svg>\n"));
        assert!(out.contains(r#"width="1152" height="720" viewBox="0 0 1152 720""#), "120×9.6 and 36×20");
        // The header's ink runs from the badge to the gear: nothing before the first column, and the
        // gear's own run inside the frame's last four cells — the header's contract, and what a click
        // on it is measured against.
        let runs: Vec<(f64, f64)> = out
            .lines()
            .filter(|line| line.contains(r#"y="15.4""#))
            .filter_map(|line| {
                let attr = |name: &str| -> Option<f64> {
                    let at = line.find(&format!(r#"{name}=""#))? + name.len() + 2;
                    line[at..].split('"').next()?.parse().ok()
                };
                Some((attr("x")?, attr("textLength")?))
            })
            .collect();
        assert!(!runs.is_empty(), "the header row is drawn at all");
        let left = runs.iter().map(|run| run.0).fold(f64::INFINITY, f64::min);
        let right = runs.iter().map(|run| run.0 + run.1).fold(f64::NEG_INFINITY, f64::max);
        assert!(left <= CELL_W, "the badge starts at the first column, not {left}");
        assert!(right >= 1152.0 - 4.0 * CELL_W, "the gear ends at the right edge, not {right}");
        // A run is a run of *drawn* cells, so a gap between two words is no text at all: the badge's
        // own two words are two runs and only one of them is a substring of the file.
        assert!(out.contains("COMPAE"), "the header badge carries the profile's own name");
        assert!(out.matches("<text ").count() < 900, "runs, not cells: {}", out.matches("<text ").count());
        assert!(!out.contains("&&"), "the file is XML, not a script");
    }

    /// The two things a reader's font would otherwise decide for us.
    #[test]
    fn a_run_carries_its_length_and_its_ink() {
        let state = chat();
        let out = svg(&render(&state, 36));
        // `◆` is gold in the HUD's palette, and the timing that follows a spoken reply is dim.
        assert!(out.contains("#ffbe3c"), "the assistant's marker keeps the palette's gold");
        // The chat's assistant entries are cyan; the user's own are white on the dim `TÚ`.
        assert!(out.contains("#00d2ff"), "the reply's own ink");
    }

    #[test]
    fn the_escape_is_what_a_browser_needs() {
        assert_eq!(escape("F9/Esc & <token>"), "F9/Esc &amp; &lt;token&gt;");
    }

    /// A `textLength` of zero would collapse a run to nothing; a cell is always worth a cell.
    #[test]
    fn reset_is_the_sheet_and_a_cell_of_it_is_not_a_rect() {
        assert_eq!(hex(Color::Reset, BASE_BG), BASE_BG);
        assert_eq!(hex(Color::Reset, BASE_FG), BASE_FG);
        assert_eq!(hex(Color::Rgb(0, 210, 255), BASE_FG), "#00d2ff");
        assert_eq!(hex(Color::Rgb(1, 2, 3), BASE_FG), "#010203");
    }
}
