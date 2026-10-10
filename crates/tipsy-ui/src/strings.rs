//! Interface strings. Spanish for Spanish-speaking personas, English for everyone else.

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
    pub cleared: &'static str,
    pub help: &'static str,
    pub weekdays: [&'static str; 7],
    pub months: [&'static str; 12],
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
    cleared: "Conversación borrada",
    help: " Enter preguntar · !texto recitar · Tab micro · F4 borrar · ←→ chat/red · Esc salir",
    weekdays: ["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"],
    months: ["enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre"],
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
    cleared: "Conversation cleared",
    help: " Enter ask · !text recite · Tab mic · F4 clear · ←→ chat/network · Esc quit",
    weekdays: ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
    months: ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"],
};

pub fn strings(lang: UiLanguage) -> &'static Strings {
    match lang {
        UiLanguage::Es => &ES,
        UiLanguage::En => &EN,
    }
}
