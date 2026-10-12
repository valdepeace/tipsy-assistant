//! Interface strings. Spanish for Spanish-speaking profiles, English for everyone else.

use tipsy_protocol::UiLanguage;

pub struct Strings {
    pub local_time: &'static str,
    pub core: &'static str,
    pub voice: &'static str,
    pub system: &'static str,
    pub chat: &'static str,
    pub network: &'static str,
    pub ask: &'static str,
    pub clear: &'static str,
    pub placeholder: &'static str,
    pub you: &'static str,
    pub ready: &'static str,
    pub listening: &'static str,
    pub transcribing: &'static str,
    pub thinking: &'static str,
    pub speaking: &'static str,
    /// The header indicator for the whole of a live call (Plan 2C).
    pub live: &'static str,
    /// Why a call did not start while `[realtime] enabled` is false.
    pub live_off: &'static str,
    pub cleared: &'static str,
    pub help: &'static str,
    pub weekdays: [&'static str; 7],
    pub months: [&'static str; 12],
    pub cfg_title: &'static str,
    pub cfg_providers: &'static str,
    pub cfg_listening: &'static str,
    pub cfg_profiles: &'static str,
    pub cfg_server: &'static str,
    pub cfg_connector: &'static str,
    pub cfg_base_url: &'static str,
    pub cfg_model: &'static str,
    pub cfg_key_name: &'static str,
    pub cfg_test: &'static str,
    pub cfg_mode: &'static str,
    pub cfg_silence_ms: &'static str,
    pub cfg_mic_gain: &'static str,
    pub cfg_mic_device: &'static str,
    pub cfg_profile: &'static str,
    pub cfg_name: &'static str,
    pub cfg_language: &'static str,
    pub cfg_prompt: &'static str,
    pub cfg_voice_instructions: &'static str,
    pub cfg_greeting: &'static str,
    pub cfg_wake_word: &'static str,
    pub cfg_voices: &'static str,
    pub cfg_bind: &'static str,
    pub cfg_lan: &'static str,
    pub cfg_token: &'static str,
    pub cfg_regenerate: &'static str,
    pub cfg_save: &'static str,
    pub cfg_dirty: &'static str,
    pub cfg_unset: &'static str,
    pub cfg_on: &'static str,
    /// The LAN switch's other position. It is not [`Strings::cfg_unset`]: a plain off switch that
    /// reads "unset" tells the user nothing was chosen, which is a different state.
    pub cfg_off: &'static str,
    pub cfg_help: &'static str,
    pub cfg_help_editing: &'static str,
    pub cfg_cancel: &'static str,
    pub cfg_value_not_whole: &'static str,
    pub kind_openai_compat: &'static str,
    pub kind_gemini: &'static str,
    pub kind_elevenlabs: &'static str,
    pub mode_push: &'static str,
    pub mode_auto: &'static str,
    pub mode_hands_free: &'static str,
    /// The HUD's notice for `hands_free`: the third mode the configuration accepts and the one this
    /// plan does not implement. It says what is missing rather than staying quiet.
    pub hands_free_unimplemented: &'static str,
    /// The HUD's notice for a binary whose embed holds no bundle (spec §5.4). It names the
    /// user-visible consequence, not the build command: the command belongs on the page the routes
    /// serve, and it is long.
    pub web_unavailable: &'static str,
    /// The login widget's one line, when it has nothing else to say (spec §6.1, §8).
    pub login_prompt: &'static str,
    /// What the transcriber heard, with the answer in `{text}`: a misheard accent is visible rather
    /// than mysterious.
    pub login_heard: &'static str,
    /// The refusal, and the only thing a wrong answer ever says.
    pub login_refusal: &'static str,
    /// The door opening, with the matched entry's name in `{name}`.
    pub login_pass: &'static str,
    /// The same, for an entry that carries no name.
    pub login_pass_anon: &'static str,
    /// `ErrorCode::NothingHeard` on the screen: the UI's own wording, because the server's message for
    /// it is English.
    pub login_nothing_heard: &'static str,
    /// A microphone the screen wanted and did not get: typing always works (spec §9).
    pub login_no_mic: &'static str,
    /// The widget's title, right-aligned and red while the capture is open.
    pub login_rec: &'static str,
    /// The keys, on the widget's bottom border.
    pub login_keys: &'static str,
    /// The same border from the third refusal in a row: the escape hatch that keeps a locked-out owner
    /// from being stuck (spec §6, L8).
    pub login_keys_lost: &'static str,
    /// `ErrorCode::ProtocolVersion`: the handshake refused because the two ends do not speak the same
    /// protocol. The number itself is deliberately not spelled here — the tables hold `&'static str`,
    /// so a `{PROTOCOL_VERSION}` could not be interpolated, and the version belongs in a log line.
    pub err_protocol_version: &'static str,
    /// `ErrorCode::TokenRequired`: the server is exposed and the client presented no token.
    pub err_token_required: &'static str,
    /// `ErrorCode::TokenInvalid`: the token presented is not this server's. The refused token is never
    /// echoed back.
    pub err_token_invalid: &'static str,
    /// The handshake's fallback sentence, for an ending that is not one of the three above.
    pub err_handshake: &'static str,
    /// The browser client lost its socket and is retrying. v1 spec §12: "the client reconnects with
    /// backoff". **The native `--connect` bridge does not use it** — its own line is an
    /// `Event::Error` message built in `client/bridge.rs`, which is a different sink — so this pair is
    /// the web client's, and it is here rather than in `tipsy-web` because `UILanguage`'s table is
    /// the project's only i18n mechanism.
    pub web_offline: &'static str,
    /// The socket is up again, which is what the session's own `Welcome` proves.
    pub web_online: &'static str,
    /// The browser gate's button. The v1 spec asks for it in as many words: "In the browser, audio
    /// capture requires a user gesture; the web client shows a 'press to start' screen until then"
    /// (`docs/superpowers/specs/2026-10-10-tipsy-compadre-design.md:193-194`).
    pub mic_start: &'static str,
    /// The line beside that button while the microphone can be offered at all.
    pub mic_hint: &'static str,
    /// `navigator.mediaDevices` is not there at all — it is `undefined` in an insecure context, which
    /// is why this is a *capability* reason and not a permission one, and why it is the message that
    /// tells the user to reach the page on loopback or over a certificate the browser trusts (MDN,
    /// quoted at `ratzilla-and-wasm-audio.md` §4.1; web-tier spec §9.5).
    pub mic_no_api: &'static str,
    /// The capture opened: the frames are flowing.
    pub mic_granted: &'static str,
    /// "Listening disabled with a warning; typing still works" — v1 spec §12 `:418`.
    pub mic_denied: &'static str,
    pub mic_nodevice: &'static str,
    pub mic_busy: &'static str,
    /// The rest of the ways a capture fails — the worklet refused to load, the browser ignored the
    /// constraints, a transient rejection. One message, because the user's next move is the same for
    /// all three, and because "everything else" is not a set worth naming one string per member.
    pub mic_failed: &'static str,
}

pub static ES: Strings = Strings {
    local_time: "HORA LOCAL",
    core: "NÚCLEO",
    voice: "VOZ",
    system: "SISTEMA",
    chat: "CHAT",
    network: "RED",
    ask: "PREGUNTA",
    clear: "✗ Borrar",
    placeholder: "Pregúntale algo... (Tab para hablarle)",
    you: "tú",
    ready: "Listo, quillo",
    listening: "Escuchando... (Tab para terminar)",
    transcribing: "Transcribiendo...",
    thinking: "Pensando...",
    speaking: "Hablando...",
    live: "EN VIVO",
    live_off: "El modo en vivo está apagado ([realtime] enabled = false)",
    cleared: "Conversación borrada",
    help: " Enter preguntar · !texto recitar · Tab micro · F4 borrar · F10 llamada · ←→ chat/red · Esc salir",
    weekdays: ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"],
    months: ["enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre"],
    cfg_title: "CONFIGURACIÓN",
    cfg_providers: "Proveedores",
    cfg_listening: "Escucha",
    cfg_profiles: "Perfiles",
    cfg_server: "Servidor",
    cfg_connector: "Conector",
    cfg_base_url: "URL base",
    cfg_model: "Modelo",
    cfg_key_name: "Nombre de la clave",
    cfg_test: "Probar",
    cfg_mode: "Modo",
    cfg_silence_ms: "Silencio (ms)",
    cfg_mic_gain: "Ganancia del micro",
    cfg_mic_device: "Dispositivo de entrada",
    cfg_profile: "Perfil",
    cfg_name: "Nombre",
    cfg_language: "Idioma",
    cfg_prompt: "Prompt",
    cfg_voice_instructions: "Instrucciones de voz",
    cfg_greeting: "Saludo",
    cfg_wake_word: "Palabra clave",
    cfg_voices: "Voces",
    cfg_bind: "Dirección",
    cfg_lan: "Red local",
    cfg_token: "Token",
    cfg_regenerate: "Regenerar token",
    cfg_save: "Guardar",
    cfg_dirty: "sin guardar",
    cfg_unset: "sin definir",
    cfg_on: "activado",
    cfg_off: "apagado",
    cfg_help: " F9 configurar · ↑↓ fila · ←→ pestaña · Enter editar/alternar · F2 probar · F4/Ctrl+S guardar · Esc cerrar",
    cfg_help_editing: " escribiendo… · Enter aplicar · Esc descartar",
    cfg_cancel: "descartar",
    cfg_value_not_whole: "número no válido",
    kind_openai_compat: "openai_compat",
    kind_gemini: "gemini",
    kind_elevenlabs: "elevenlabs",
    mode_push: "push",
    mode_auto: "auto",
    mode_hands_free: "hands free",
    hands_free_unimplemented: "manos libres aún no está: espera a la palabra clave",
    web_unavailable: "la interfaz web no está disponible en esta compilación",
    login_prompt: "¿Y tú quién eres? Dímelo, que aquí no entra cualquiera.",
    login_heard: "Te he oído: «{text}»",
    login_refusal: "Anda, que te peinen para el lado.",
    login_pass: "Pasa, {name}.",
    login_pass_anon: "Pasa.",
    login_nothing_heard: "No te he oído. Repite, si eso.",
    login_no_mic: "Sin micro: escribe, que también vale.",
    login_rec: "● rec",
    login_keys: "Tab micro · Enter entrar · esc salir",
    login_keys_lost: "Tab micro · Enter entrar · ¿olvidado? [login] en config.toml",
    err_protocol_version: "Esta versión del cliente no habla el protocolo de este servidor.",
    err_token_required: "Este servidor está expuesto y pide un token: pásalo con --token.",
    err_token_invalid: "Ese no es el token de este servidor.",
    err_handshake: "No se pudo completar el saludo con el servidor.",
    web_offline: "Sin conexión: reintentando…",
    web_online: "Conexión restablecida",
    mic_start: "Pulsa para empezar",
    mic_hint: "El navegador pedirá permiso para el micrófono",
    mic_no_api: "Este navegador no da acceso al micrófono aquí: abre la página en localhost o por HTTPS.",
    mic_granted: "Micrófono abierto",
    mic_denied: "Micrófono denegado: revisa el permiso del sitio. Puedes escribir.",
    mic_nodevice: "No hay micrófono conectado. Puedes escribir.",
    mic_busy: "El micrófono está ocupado por otra aplicación. Puedes escribir.",
    mic_failed: "No se pudo abrir el micrófono. Puedes escribir.",
};

pub static EN: Strings = Strings {
    local_time: "LOCAL TIME",
    core: "CORE",
    voice: "VOICE",
    system: "SYSTEM",
    chat: "CHAT",
    network: "NETWORK",
    ask: "ASK",
    clear: "✗ Clear",
    placeholder: "Ask something... (Tab to talk)",
    you: "you",
    ready: "Ready",
    listening: "Listening... (Tab to stop)",
    transcribing: "Transcribing...",
    thinking: "Thinking...",
    speaking: "Speaking...",
    live: "LIVE",
    live_off: "The live mode is off ([realtime] enabled = false)",
    cleared: "Conversation cleared",
    help: " Enter ask · !text recite · Tab mic · F4 clear · F10 call · ←→ chat/network · Esc quit",
    weekdays: ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
    months: ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"],
    cfg_title: "CONFIGURATION",
    cfg_providers: "Providers",
    cfg_listening: "Listening",
    cfg_profiles: "Profiles",
    cfg_server: "Server",
    cfg_connector: "Connector",
    cfg_base_url: "Base URL",
    cfg_model: "Model",
    cfg_key_name: "Key name",
    cfg_test: "Test",
    cfg_mode: "Mode",
    cfg_silence_ms: "Silence (ms)",
    cfg_mic_gain: "Mic gain",
    cfg_mic_device: "Input device",
    cfg_profile: "Profile",
    cfg_name: "Name",
    cfg_language: "Language",
    cfg_prompt: "Prompt",
    cfg_voice_instructions: "Voice instructions",
    cfg_greeting: "Greeting",
    cfg_wake_word: "Wake word",
    cfg_voices: "Voices",
    cfg_bind: "Bind",
    cfg_lan: "LAN",
    cfg_token: "Token",
    cfg_regenerate: "Regenerate token",
    cfg_save: "Save",
    cfg_dirty: "unsaved",
    cfg_unset: "unset",
    cfg_on: "on",
    cfg_off: "off",
    cfg_help: " F9 configure · ↑↓ row · ←→ tab · Enter edit/toggle · F2 test · F4/Ctrl+S save · Esc close",
    cfg_help_editing: " typing… · Enter apply · Esc discard",
    cfg_cancel: "discard",
    cfg_value_not_whole: "not a number",
    kind_openai_compat: "openai_compat",
    kind_gemini: "gemini",
    kind_elevenlabs: "elevenlabs",
    mode_push: "push",
    mode_auto: "auto",
    mode_hands_free: "hands free",
    hands_free_unimplemented: "hands free is not implemented yet: it waits for the wake word",
    web_unavailable: "the web UI is not available in this build",
    login_prompt: "So who are you, then? Tell me — this door isn't for just anyone.",
    login_heard: "I heard: \"{text}\"",
    login_refusal: "Off you go, and get your hair combed somewhere else.",
    login_pass: "Come in, {name}.",
    login_pass_anon: "Come in.",
    login_nothing_heard: "I heard nothing. Try again, if you like.",
    login_no_mic: "No mic: type it, that works too.",
    login_rec: "● rec",
    login_keys: "Tab mic · Enter in · esc quit",
    login_keys_lost: "Tab mic · Enter in · forgotten it? [login] in config.toml",
    err_protocol_version: "This client's version does not speak this server's protocol.",
    err_token_required: "This server is exposed and asks for a token: pass it with --token.",
    err_token_invalid: "That is not this server's token.",
    err_handshake: "The handshake with the server could not be completed.",
    web_offline: "Connection lost: retrying…",
    web_online: "Connection restored",
    mic_start: "Press to start",
    mic_hint: "The browser will ask for the microphone",
    // The plan's own Step 6 English table stops one field short of the Spanish one — this line is the
    // one it lost, and it is the reason the capability probe can answer in English at all.
    mic_no_api: "This browser gives this page no microphone access: open it on localhost or over HTTPS.",
    mic_granted: "Microphone open",
    mic_denied: "Microphone denied: check the site's permission. You can still type.",
    mic_nodevice: "No microphone connected. You can still type.",
    mic_busy: "The microphone is in use by another application. You can still type.",
    mic_failed: "The microphone could not be opened. You can still type.",
};

pub fn strings(lang: UiLanguage) -> &'static Strings {
    match lang {
        UiLanguage::Es => &ES,
        UiLanguage::En => &EN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The indicator's copy exists in both languages — Review Focus 3's "the mode is never silently
    /// on" starts with a word for it in the table the user's profile selects.
    #[test]
    fn the_live_indicator_has_copy_in_both_languages() {
        assert_eq!(strings(UiLanguage::Es).live, "EN VIVO");
        assert_eq!(strings(UiLanguage::En).live, "LIVE");
        assert_ne!(strings(UiLanguage::Es).live_off, strings(UiLanguage::En).live_off);
        for table in [strings(UiLanguage::Es), strings(UiLanguage::En)] {
            assert!(!table.live_off.is_empty(), "a refusal has to say something");
            assert!(table.help.contains("F10"), "the help line names the key: {}", table.help);
        }
    }

    /// The two socket notices exist in both languages and say different things: a client that showed
    /// "connection lost" when the connection came back would be worse than silent. This is Task 3.2's
    /// pair, and the reason they are in this table rather than hard-coded in `tipsy-web`.
    #[test]
    fn the_socket_notices_have_copy_in_both_languages() {
        for table in [strings(UiLanguage::Es), strings(UiLanguage::En)] {
            assert!(!table.web_offline.is_empty(), "a lost connection has to say something");
            assert!(!table.web_online.is_empty(), "and so does a restored one");
            assert_ne!(table.web_offline, table.web_online, "the two are not the same notice");
        }
        assert_ne!(strings(UiLanguage::Es).web_offline, strings(UiLanguage::En).web_offline);
        assert_ne!(strings(UiLanguage::Es).web_online, strings(UiLanguage::En).web_online);
    }

    /// The browser gate's copy, in both tables and in both directions: the gate's own two lines, the
    /// five reasons `MicAvailability` can hand back, and the pass notice. **This is the test the plan's
    /// own Step 6 table would have failed**: its English block lists seven of the eight fields, so the
    /// `EN` table would not have compiled — and a missing `mic_no_api` is the one reason an insecure
    /// context can produce, which is exactly the state the capability probe exists for.
    #[test]
    fn the_microphone_copy_is_complete_in_both_languages() {
        for table in [strings(UiLanguage::Es), strings(UiLanguage::En)] {
            for (name, text) in [
                ("mic_start", table.mic_start),
                ("mic_hint", table.mic_hint),
                ("mic_no_api", table.mic_no_api),
                ("mic_granted", table.mic_granted),
                ("mic_denied", table.mic_denied),
                ("mic_nodevice", table.mic_nodevice),
                ("mic_busy", table.mic_busy),
                ("mic_failed", table.mic_failed),
            ] {
                assert!(!text.trim().is_empty(), "{name} has to say something");
            }
            // "typing still works" is the promise the v1 spec makes for every disabled microphone
            // (§12), so each refusal says so rather than leaving the user to guess.
            for text in [table.mic_denied, table.mic_nodevice, table.mic_busy, table.mic_failed] {
                assert!(text.contains("escribir") || text.contains("type"), "{text}");
            }
            assert!(table.mic_no_api.contains("HTTPS"), "the insecure-context reason names the fix");
        }
        assert_ne!(strings(UiLanguage::Es).mic_start, strings(UiLanguage::En).mic_start);
        assert_ne!(strings(UiLanguage::Es).mic_no_api, strings(UiLanguage::En).mic_no_api);
        assert_ne!(strings(UiLanguage::Es).mic_granted, strings(UiLanguage::En).mic_granted);
    }
}
