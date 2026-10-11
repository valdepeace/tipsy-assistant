//! The configuration screen's form view model.
//!
//! Plain values only: no `Config` (that type lives in `tipsy-compadre`, which this crate may not
//! depend on), no clock, no terminal. The native client converts `Config` ⇄ `ConfigForm` and is the
//! only thing that reads or writes the file; everything on this side is what the screen shows and
//! what a key or a click does to it.

use tipsy_protocol::{Capability, ListeningMode, UiLanguage};

use crate::state::KeyInput;
use crate::strings::{strings, Strings};

/// The keyboard's `Ctrl+S`: `KeyInput::Char` carries no modifiers, and a control character is the
/// one `char` a text row would never want, so the two chords the screen has need no new variant.
/// `Ctrl+S` saves and `Ctrl+R` regenerates the server token, and `Ctrl+R` is not a browser-reserved
/// key (spec §9 rules out `F5`, `F11` and `Ctrl+W`). The client's one-line `CONTROL` mask is
/// Task 5's; this module only handles the `KeyInput` it is handed.
pub const SAVE_KEY: char = '\u{13}';
pub const REGENERATE_KEY: char = '\u{12}';

/// The silence threshold a form accepts, in milliseconds: `silence_ms` is read by the VAD, which
/// cannot hear a pause shorter than this or wait longer than that.
pub const SILENCE_MS_MIN: u32 = 100;
pub const SILENCE_MS_MAX: u32 = 30_000;
/// The gain the form accepts. Above this a quiet room is already clipping.
pub const MIC_GAIN_MIN: f32 = 0.0;
pub const MIC_GAIN_MAX: f32 = 30.0;

/// The scalar rows the active profile contributes to the Profiles tab, one per editable
/// `ProfileForm` field. The profile-list rows and the voice entries are counted separately, because
/// how many of those there are is a property of the form: see [`ConfigForm::profile_row_count`].
pub const PROFILE_FIELD_COUNT: usize = 7;
/// The Server tab's rows are `bind`, `lan`, `token`, `regenerate` and Save, so its Save row is the
/// fifth and last one. Task 4's tests name it; a tab's layout is not re-derived per task.
pub const SERVER_ROW_SAVE: usize = 4;
/// The Server tab's other action row: the one that mints a fresh token.
pub const SERVER_ROW_REGENERATE: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigTab {
    Providers,
    Listening,
    Profiles,
    Server,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeKind {
    Ok,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfigEffect {
    /// Declared because the plan's signature fixes it, and never constructed: `apply` returns an
    /// empty `Vec` for a no-op.
    None,
    Save,
    Test(Capability),
    RegenerateToken,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FormRow {
    /// From the string table: a label is a `&'static str`, so it cannot be built at runtime and
    /// cannot escape `Strings`.
    pub label: &'static str,
    pub value: String,
    pub editable: bool,
    pub is_active: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderForm {
    pub capability: Capability,
    pub kind: String,
    pub base_url: String,
    pub model: String,
    pub key_name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListeningForm {
    pub mode: ListeningMode,
    pub silence_ms: String,
    pub mic_gain: String,
    /// `None` is the system default device and hides the row; `Some("")` is a named device whose
    /// name is empty, which is a different (and useless) value, so it keeps its row. `Config` holds
    /// the same distinction.
    pub mic_device: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerForm {
    pub bind: String,
    pub lan: bool,
    pub token: String,
}

/// The `[realtime]` section, carried through the screen but never drawn on it (Plan 2C Task 1).
///
/// The section is edited in `config.toml` — realtime is a mode of the client, and no tab of this
/// screen shows it — but the native client rebuilds the whole configuration from this form on a
/// save, so a section the form did not carry would come back at its defaults, resetting `enabled`,
/// the model and the voice the first time anything on the screen was saved. Nothing here is
/// editable, which is why the values keep the file's own types instead of the text buffers an
/// editable row needs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RealtimeForm {
    pub enabled: bool,
    pub model: String,
    /// The key's *name*, never the key: the same rule as `ProviderForm::key_name`.
    pub key_name: String,
    pub voice: String,
    pub max_minutes: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfileForm {
    pub id: String,
    pub name: String,
    pub language: String,
    pub prompt: String,
    pub voice_instructions: String,
    pub greeting: String,
    pub wake_word: String,
    /// Sorted by connector name: a map's order is not a screen order, and the row index `field` is
    /// what the clicks name back. `Config`'s `BTreeMap` arrives here in exactly this order.
    pub voices: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigForm {
    pub tab: ConfigTab,
    /// The focused row inside the current tab, an index into [`ConfigForm::rows`].
    pub field: usize,
    /// The text being typed right now. It starts **empty** and replaces the row's value when it is
    /// committed, because the cursor is at the end of the buffer and there is no second cursor.
    pub editing: Option<String>,
    pub active_profile: String,
    pub providers: Vec<ProviderForm>,
    pub listening: ListeningForm,
    /// Carried, never drawn: see [`RealtimeForm`].
    pub realtime: RealtimeForm,
    pub profiles: Vec<ProfileForm>,
    pub server: ServerForm,
    pub notice: Option<(NoticeKind, String)>,
    pub dirty: bool,
    /// True once [`ConfigForm::mark_filled`] has been called with a form the client converted from
    /// the file. [`ConfigForm::default`] is the placeholder the screen opens on before the client
    /// has answered, and a placeholder is never saveable: writing it would replace a working
    /// `config.toml` with blank base URLs and models.
    pub(crate) filled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigAction {
    Open,
    Close,
    /// The screen's door, as a control reports it. The header's mode indicator and gear are the only
    /// controls that open the screen, and while the screen is up they are *behind* it: a click on
    /// those columns is a click on whatever the screen draws there. Reporting `Toggle` rather than a
    /// key is what keeps a click from being read as `F9` by a screen that answers `F9` by closing.
    Toggle,
    NextTab,
    PrevTab,
    NextField,
    PrevField,
    BeginEdit,
    Push(char),
    Backspace,
    CommitEdit,
    SelectProfile(usize),
    /// A click on a tab label names the tab it landed on, rather than a keystroke the focused row
    /// might reinterpret. The keyboard reaches the same tabs through [`ConfigAction::NextTab`] and
    /// [`ConfigAction::PrevTab`], and through the `F` keys `action_for` maps to this action.
    SelectTab(ConfigTab),
    /// A click on a row names the row it landed on. `Toggle` and `BeginEdit` act on the *focused*
    /// row, which is what a keyboard means by them; a click means the row under the pointer, and
    /// the form is what turns the index into the row's own action.
    FocusRow(usize),
    Save,
    Test,
    RegenerateToken,
}

/// What one key did: the action it was translated into, if any, and the effects that action had.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyOutcome {
    pub action: Option<ConfigAction>,
    pub effects: Vec<ConfigEffect>,
}

impl Default for ConfigForm {
    fn default() -> Self {
        Self {
            tab: ConfigTab::Providers,
            field: 0,
            editing: None,
            active_profile: "compae".into(),
            // One placeholder group per capability, so the Providers tab draws a Connector, a
            // Base URL, a Model and a Test row the moment the screen opens, before the client has
            // answered. A form whose list is empty draws no rows at all on its own first tab, which
            // is not a screen a user can work in. The native client replaces every field of this
            // form from the real configuration (Task 6) *and marks it filled*, so what these hold
            // matters only until it does — and a form that was never filled refuses to save.
            providers: [Capability::Chat, Capability::Speech, Capability::Transcribe]
                .into_iter()
                .map(|capability| ProviderForm {
                    capability,
                    kind: strings(UiLanguage::Es).kind_openai_compat.to_owned(),
                    base_url: String::new(),
                    model: String::new(),
                    key_name: String::new(),
                })
                .collect(),
            listening: ListeningForm {
                mode: ListeningMode::Push,
                silence_ms: "1000".into(),
                mic_gain: "12.0".into(),
                mic_device: None,
            },
            // The placeholder form draws no realtime row either, so its empty carry is never seen;
            // the native client fills it from the file on the only form that can be saved.
            realtime: RealtimeForm::default(),
            // One placeholder profile, so the rows below have something to index. The native client
            // replaces every field of this form from the real configuration (Task 6).
            profiles: vec![ProfileForm {
                id: "compae".into(),
                name: "El Compae".into(),
                language: "es-ES".into(),
                ..ProfileForm::default()
            }],
            server: ServerForm { bind: "127.0.0.1:7070".into(), lan: false, token: String::new() },
            notice: None,
            dirty: false,
            filled: false,
        }
    }
}

impl ConfigForm {
    /// The rows the active tab shows, in the order `field` counts them. The renderer draws this
    /// list and the click areas are the rectangles of this same list, so the two cannot disagree.
    pub fn rows(&self) -> Vec<FormRow> {
        match self.tab {
            ConfigTab::Providers => self.provider_rows(),
            ConfigTab::Listening => self.listening_rows(),
            ConfigTab::Profiles => self.profile_rows(),
            ConfigTab::Server => self.server_rows(),
        }
    }

    /// The labels of [`ConfigForm::rows`], for the tests that check every one of them against the
    /// string tables.
    pub fn row_labels(&self) -> Vec<&'static str> {
        self.rows().into_iter().map(|row| row.label).collect()
    }

    /// The row where the Profiles tab's voice table starts, computed from the profiles actually
    /// shown: one list row each, then the active profile's field rows. It is a function and never a
    /// constant, because the list's length is what moves the table down.
    pub fn voice_label_row(&self) -> usize {
        self.profiles.len() + PROFILE_FIELD_COUNT
    }

    /// How many rows the Profiles tab shows: the profile list, the active profile's field rows and
    /// its voice entries, two rows per entry (the connector and the voice name). `profile_rows`
    /// builds exactly this list, so the count cannot drift from what is drawn.
    pub fn profile_row_count(form: &ConfigForm) -> usize {
        form.profiles.len() + PROFILE_FIELD_COUNT + 2 * active_voices(form).len()
    }

    /// The title of a tab, read from whichever table the caller's language selected. It reads
    /// nothing from the form, and takes `self` so the screen can ask the form it already has.
    pub fn tab_title(&self, tab: ConfigTab, strings: &Strings) -> &'static str {
        match tab {
            ConfigTab::Providers => strings.cfg_providers,
            ConfigTab::Listening => strings.cfg_listening,
            ConfigTab::Profiles => strings.cfg_profiles,
            ConfigTab::Server => strings.cfg_server,
        }
    }

    /// True while the server token is unset: the empty string is the state, not a stored empty
    /// token.
    pub fn unset(&self) -> bool {
        self.server.token.trim().is_empty()
    }

    /// True when this form carries what the client read from `config.toml`. The placeholder the
    /// screen opens on is not filled, and an unfilled form refuses to save: a read that failed
    /// leaves an empty template on screen, and `Ctrl+S` must not write that over the user's file.
    pub fn filled(&self) -> bool {
        self.filled
    }

    /// Marks the form as carrying a configuration the client actually read. Called by the client
    /// when it hands the converted form to [`crate::state::UiState::show_config_form`]; a form that
    /// was never marked this way is the placeholder and cannot be saved.
    pub fn mark_filled(&mut self) {
        self.filled = true;
    }

    /// Every screen string the form can show, so a test can prove both tables carry all of them
    /// without a hand-kept checklist. The keys a state happens to keep off screen are included on
    /// purpose: a table that lacks the string the next keystroke needs is the bug this prevents.
    pub fn string_keys(&self) -> Vec<&'static str> {
        let es = strings(UiLanguage::Es);
        let en = strings(UiLanguage::En);
        vec![
            es.cfg_title,
            es.cfg_providers,
            es.cfg_listening,
            es.cfg_profiles,
            es.cfg_server,
            es.cfg_connector,
            es.cfg_base_url,
            es.cfg_model,
            es.cfg_key_name,
            es.cfg_test,
            es.cfg_mode,
            es.cfg_silence_ms,
            es.cfg_mic_gain,
            es.cfg_mic_device,
            es.cfg_profile,
            es.cfg_name,
            es.cfg_language,
            es.cfg_prompt,
            es.cfg_voice_instructions,
            es.cfg_greeting,
            es.cfg_wake_word,
            es.cfg_voices,
            es.cfg_bind,
            es.cfg_lan,
            es.cfg_token,
            es.cfg_regenerate,
            es.cfg_save,
            es.cfg_dirty,
            es.cfg_unset,
            es.cfg_on,
            es.cfg_off,
            es.cfg_cancel,
            es.cfg_help,
            es.cfg_help_editing,
            es.cfg_value_not_whole,
            es.kind_openai_compat,
            es.kind_gemini,
            es.kind_elevenlabs,
            es.mode_push,
            es.mode_auto,
            es.mode_hands_free,
            en.cfg_cancel,
            en.cfg_off,
            en.cfg_help,
            en.cfg_help_editing,
        ]
    }

    /// One key, turned into the action it means — and *not* applied. `UiState` needs exactly this:
    /// the translation is the form's own knowledge (which row a key reaches, whether a buffer is
    /// open), while applying the action is the native client's job, because only the client can
    /// write the file. Splitting it here is what keeps one key from being reduced twice, and what
    /// keeps the translation in one place for [`ConfigForm::key`] and the screen's dispatcher alike.
    pub fn action_for(&self, key: KeyInput) -> Option<ConfigAction> {
        match key {
            // A control character is the one `char` a text row wants none of: the two chords the
            // screen has arrive as their own control characters, so no variant is needed for either.
            KeyInput::Char(SAVE_KEY) => Some(ConfigAction::Save),
            KeyInput::Char(REGENERATE_KEY) => Some(ConfigAction::RegenerateToken),
            KeyInput::Char(c) => {
                if self.editing.is_some() {
                    Some(ConfigAction::Push(c))
                } else {
                    // Typing outside an edit is not a text entry: a printable key is not a shortcut,
                    // and every shortcut the form has is a control character or a named key. The
                    // space bar is not a second `Enter` either: the help line used to promise one
                    // and the reducer never had it, so the copy lost the promise instead.
                    None
                }
            }
            KeyInput::Backspace => self.editing.is_some().then_some(ConfigAction::Backspace),
            KeyInput::Enter => {
                Some(if self.editing.is_some() { ConfigAction::CommitEdit } else { self.enter_action() })
            }
            KeyInput::Tab | KeyInput::Right => {
                if self.editing.is_some() { None } else { Some(ConfigAction::NextTab) }
            }
            KeyInput::Left => {
                if self.editing.is_some() { None } else { Some(ConfigAction::PrevTab) }
            }
            KeyInput::Up => self.editing.is_none().then_some(ConfigAction::PrevField),
            KeyInput::Down => self.editing.is_none().then_some(ConfigAction::NextField),
            // `F2` and `F4` are the two buttons the action line draws, so the keyboard has the same
            // two actions the mouse reaches. `F4` saves from any tab; `F2` probes the focused
            // Providers group and is inert elsewhere, because there is nothing else to probe. Neither
            // reaches into an open buffer, for the same reason no other action does: the buffer is
            // the only thing on screen that can be changed, and Enter applies it, Esc drops it.
            //
            // The screen's tabs answer `F5`, `F6`, `F8` and `F10` — never `F9`, which is the door in
            // and out of the screen (`UiState::on_key`) and must not also be a tab.
            KeyInput::F(2) => {
                (self.editing.is_none() && self.tab == ConfigTab::Providers)
                    .then_some(ConfigAction::Test)
            }
            KeyInput::F(4) => self.editing.is_none().then_some(ConfigAction::Save),
            KeyInput::F(5) => {
                self.editing.is_none().then_some(ConfigAction::SelectTab(ConfigTab::Providers))
            }
            KeyInput::F(6) => {
                self.editing.is_none().then_some(ConfigAction::SelectTab(ConfigTab::Listening))
            }
            KeyInput::F(8) => {
                self.editing.is_none().then_some(ConfigAction::SelectTab(ConfigTab::Profiles))
            }
            KeyInput::F(10) => {
                self.editing.is_none().then_some(ConfigAction::SelectTab(ConfigTab::Server))
            }
            // Esc names itself in every state: the screen's close, and the buffer's discard. A key
            // a user presses to get out must never be silently inert, and `Close` is what the
            // outcome reports either way; the buffer is gone by the time the caller sees it.
            KeyInput::Esc => Some(ConfigAction::Close),
            KeyInput::F(_) => None,
        }
    }

    /// One key, turned into the action it means and applied. The form has no modifier of its own:
    /// the two chords arrive as their control characters, and everything else is a named key.
    pub fn key(&mut self, key: KeyInput) -> KeyOutcome {
        match self.action_for(key) {
            Some(action) => KeyOutcome { action: Some(action), effects: self.apply(action) },
            None => KeyOutcome { action: None, effects: Vec::new() },
        }
    }

    /// What an action does. Only the four effects the screen has are ever raised; everything else
    /// returns an empty vector rather than [`ConfigEffect::None`].
    pub fn apply(&mut self, action: ConfigAction) -> Vec<ConfigEffect> {
        // Closing is the one action every state understands: it is what `Esc` means, and the buffer
        // goes with the screen, because the screen is what owns it. Every other action takes the
        // tab path below, which is what keeps a stray `Save` or `Toggle` from landing in the middle
        // of a text edit.
        if action == ConfigAction::Close {
            self.editing = None;
            return Vec::new();
        }
        if self.editing.is_some() {
            return self.edit_action(action);
        }
        match action {
            ConfigAction::NextTab => {
                self.switch_tab(1);
                Vec::new()
            }
            ConfigAction::PrevTab => {
                self.switch_tab(-1);
                Vec::new()
            }
            ConfigAction::SelectTab(tab) => {
                self.tab = tab;
                self.field = 0;
                Vec::new()
            }
            ConfigAction::NextField => {
                self.next_field();
                Vec::new()
            }
            ConfigAction::PrevField => {
                self.prev_field();
                Vec::new()
            }
            ConfigAction::FocusRow(index) => self.focus_row(index),
            ConfigAction::BeginEdit => {
                if self.focused_editable() {
                    self.editing = Some(String::new());
                }
                Vec::new()
            }
            ConfigAction::Toggle => self.toggle(),
            ConfigAction::SelectProfile(index) => {
                self.select_profile(index);
                Vec::new()
            }
            ConfigAction::Save => {
                // An unfilled form is the placeholder the screen opens on, not the user's
                // configuration: saving it would write blank base URLs and models over a working
                // file. It refuses, and refuses quietly — there is nothing the user did wrong.
                if !self.filled {
                    return Vec::new();
                }
                self.dirty = true;
                vec![ConfigEffect::Save]
            }
            ConfigAction::Test => {
                // The Test button probes the group the focused row belongs to, whatever row of the
                // group is focused: the button acts on the connector the rows above it describe.
                if self.tab == ConfigTab::Providers {
                    vec![ConfigEffect::Test(self.provider_group())]
                } else {
                    Vec::new()
                }
            }
            ConfigAction::RegenerateToken => {
                // Regenerating mints a token into the row and writes nothing; it stays reachable on
                // an unfilled form because the frozen HUD test pins it that way, and a token that is
                // only ever saved by an explicit `Save` cannot damage the file. The save itself is
                // the guard (finding G), and it refuses an unfilled form.
                self.dirty = true;
                vec![ConfigEffect::RegenerateToken]
            }
            // The half-typed value is the edit, and it goes with the screen: the buffer is dropped
            // whatever it held, and nothing that was typed was written into any row. `Open` is the
            // client's, not the reducer's — the screen is created with the form, so there is nothing
            // here for it to change, and `Toggle` is the state's for the same reason. `Push`,
            // `Backspace` and `CommitEdit` reach the form only while `editing` is set, in
            // `edit_action` above.
            ConfigAction::Open
            | ConfigAction::Close
            | ConfigAction::Push(_)
            | ConfigAction::Backspace
            | ConfigAction::CommitEdit => Vec::new(),
        }
    }

    /// The editing reducer: while a buffer is open, only the buffer's own actions mean anything.
    /// The parsing actions are deliberately inert here, which is also why an action the screen
    /// raises with no buffer (a `Save` a Task 5 dispatch sends mid-edit) is not a way out.
    fn edit_action(&mut self, action: ConfigAction) -> Vec<ConfigEffect> {
        match action {
            ConfigAction::Push(c) => {
                if let Some(buffer) = self.editing.as_mut() {
                    buffer.push(c);
                }
            }
            ConfigAction::Backspace => {
                if let Some(buffer) = self.editing.as_mut() {
                    buffer.pop();
                }
            }
            ConfigAction::CommitEdit => self.commit_edit(),
            // A tab switch, a row move or a parse while the buffer is open is a no-op rather than a
            // half-applied edit: the buffer is the only thing on screen that can be changed, and
            // closing it is the one way out.
            ConfigAction::Open
            | ConfigAction::Close
            | ConfigAction::Toggle
            | ConfigAction::NextTab
            | ConfigAction::PrevTab
            | ConfigAction::SelectTab(_)
            | ConfigAction::NextField
            | ConfigAction::PrevField
            | ConfigAction::FocusRow(_)
            | ConfigAction::BeginEdit
            | ConfigAction::SelectProfile(_)
            | ConfigAction::Save
            | ConfigAction::Test
            | ConfigAction::RegenerateToken => {}
        }
        Vec::new()
    }

    /// Which action `Enter` takes on the focused row: an editable row opens its buffer, and every
    /// other row is actuated. A row index that is past the end of the list — a form with no profiles
    /// at all, say — actuates nothing rather than panicking.
    fn enter_action(&self) -> ConfigAction {
        if self.focused_editable() { ConfigAction::BeginEdit } else { ConfigAction::Toggle }
    }

    /// True when the focused row is a row that holds text.
    fn focused_editable(&self) -> bool {
        self.row_editable(self.field)
    }

    /// True when the row at `index` holds text. A row index the tab no longer has is not editable
    /// rather than a panic: a click from a frame that no longer matches the form is a no-op.
    fn row_editable(&self, index: usize) -> bool {
        self.field_rows().get(index).map(|row| row.editable).unwrap_or(false)
    }

    /// Moves the keyboard focus to the row a click named, and does what that row's own `Enter`
    /// would do. A click is a pointer, not a keystroke: it must reach the row under it, never
    /// whichever row happened to be focused, and a row that is not text opens no buffer.
    ///
    /// A profile-list row is the one non-text row that is not a switch: a click on it chooses that
    /// profile, which is the only way to change the active profile from the screen (`SelectProfile`
    /// carries the index, and a click names it honestly).
    ///
    /// It returns what that row's own action raised, because a button row clicked is the button
    /// pressed: the Test, Regenerate and Save rows are rows *and* the actions the action line draws,
    /// so a click that reached them has to raise the same effect `Enter` on them does. Dropping it
    /// here is what would make a click on the Test row focus a row and probe nothing.
    fn focus_row(&mut self, index: usize) -> Vec<ConfigEffect> {
        match self.field_rows().get(index) {
            // A row the tab no longer has — a click from a frame older than the form — is a no-op.
            None => Vec::new(),
            Some(row) if row.editable => {
                self.field = index;
                self.editing = Some(String::new());
                Vec::new()
            }
            Some(_) => {
                self.field = index;
                if self.tab == ConfigTab::Profiles && index < self.profiles.len() {
                    self.select_profile(index);
                    Vec::new()
                } else {
                    self.activate()
                }
            }
        }
    }

    /// The action a non-editable row takes when it is actuated, whether that came from `Enter` or
    /// from a click. Only the rows that are buttons raise anything.
    fn activate(&mut self) -> Vec<ConfigEffect> {
        match self.tab {
            ConfigTab::Providers => {
                if self.is_test_row() {
                    return vec![ConfigEffect::Test(self.provider_group())];
                }
                Vec::new()
            }
            ConfigTab::Server => match self.field {
                1 => {
                    self.server.lan = !self.server.lan;
                    self.dirty = true;
                    Vec::new()
                }
                SERVER_ROW_REGENERATE => {
                    self.dirty = true;
                    vec![ConfigEffect::RegenerateToken]
                }
                // The Save row is the Save button: it refuses an unfilled form for the same reason
                // the action does, and the two are the only two doors onto a write.
                SERVER_ROW_SAVE if self.filled => {
                    self.dirty = true;
                    vec![ConfigEffect::Save]
                }
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// `Toggle` is `activate` for the rows the keyboard reaches, and it is the only cycling key:
    /// Listening's mode, Server's LAN, and the Test button as a row. A row that is neither raises
    /// nothing at all.
    fn toggle(&mut self) -> Vec<ConfigEffect> {
        match self.tab {
            ConfigTab::Listening => {
                if self.field == 0 {
                    self.listening.mode = match self.listening.mode {
                        ListeningMode::Push => ListeningMode::Auto,
                        ListeningMode::Auto => ListeningMode::HandsFree,
                        ListeningMode::HandsFree => ListeningMode::Push,
                    };
                    self.dirty = true;
                }
                Vec::new()
            }
            ConfigTab::Server => self.activate(),
            ConfigTab::Providers => self.activate(),
            ConfigTab::Profiles => Vec::new(),
        }
    }

    /// Commits the buffer into the focused row and marks the form changed. A row whose value would
    /// not parse is refused instead, keeping what it had.
    fn commit_edit(&mut self) {
        let Some(buffer) = self.editing.take() else { return };
        let ok = match self.tab {
            ConfigTab::Providers => {
                if !self.focused_editable() {
                    false
                } else {
                    self.write_provider(buffer)
                }
            }
            ConfigTab::Listening => match self.field {
                1 => self.set_number(SILENCE_MS_MIN, SILENCE_MS_MAX, buffer),
                2 => self.set_gain(buffer),
                3 => {
                    self.listening.mic_device = if buffer.is_empty() { None } else { Some(buffer) };
                    true
                }
                _ => false,
            },
            ConfigTab::Profiles => self.commit_profile(buffer),
            ConfigTab::Server => {
                if self.field == 0 { self.server.bind = buffer; true } else { false }
            }
        };
        if ok {
            self.dirty = true;
        }
    }

    /// Commits `buffer` into the focused Profiles row. The row indices are exactly the order
    /// [`ConfigForm::profile_rows`] emits them in: the profile list first, then the active
    /// profile's scalar fields as name, language, prompt, voice instructions, greeting, wake word
    /// and last the id, then two rows per voice entry.
    fn commit_profile(&mut self, buffer: String) -> bool {
        let list_len = self.profiles.len();
        if self.field < list_len {
            // The list rows are not text: a click on one selects it, it does not edit it.
            return false;
        }
        if self.field < list_len + PROFILE_FIELD_COUNT {
            return match self.field - list_len {
                0 => self.fill(|p| Some(&mut p.name), buffer),
                1 => self.fill(|p| Some(&mut p.language), buffer),
                2 => self.fill(|p| Some(&mut p.prompt), buffer),
                3 => self.fill(|p| Some(&mut p.voice_instructions), buffer),
                4 => self.fill(|p| Some(&mut p.greeting), buffer),
                5 => self.fill(|p| Some(&mut p.wake_word), buffer),
                6 => self.write_active_id(buffer),
                _ => false,
            };
        }
        // Two rows per voice entry: the connector (a list, not text) and the name. Only the name
        // row commits, so an even offset is the connector and writes nothing.
        let offset = self.field - list_len - PROFILE_FIELD_COUNT;
        if offset.is_multiple_of(2) {
            return false;
        }
        let active = self.active_profile.clone();
        self.profiles
            .iter_mut()
            .find(|p| p.id == active)
            .and_then(|p| p.voices.get_mut(offset / 2))
            .map(|voice| voice.1 = buffer)
            .is_some()
    }

    /// Writes `value` into the focused Providers row: the group is `field / PROVIDER_ROW_COUNT` and
    /// the column tells which of its text fields the row is.
    fn write_provider(&mut self, value: String) -> bool {
        let group = self.field / PROVIDER_ROW_COUNT;
        let column = self.field % PROVIDER_ROW_COUNT;
        match self.providers.get_mut(group) {
            Some(provider) => {
                match column {
                    1 => provider.base_url = value,
                    2 => provider.model = value,
                    3 => provider.key_name = value,
                    _ => return false,
                }
                true
            }
            None => false,
        }
    }

    /// Writes `value` into the active profile's **id**: the last of the seven scalar rows, and the
    /// only one that renames the profile rather than its copy.
    fn write_active_id(&mut self, value: String) -> bool {
        let active = self.active_profile.clone();
        let Some(profile) = self.profiles.iter_mut().find(|p| p.id == active) else { return false };
        profile.id = value.clone();
        // Renaming the active profile must not switch the assistant to another one.
        self.active_profile = value;
        true
    }

    /// Writes `value` into the active profile's field the selector names. A form whose active
    /// profile is no longer in the list writes nothing rather than guessing which one it meant.
    fn fill(
        &mut self,
        selector: impl FnOnce(&mut ProfileForm) -> Option<&mut String>,
        value: String,
    ) -> bool {
        let active = self.active_profile.clone();
        match self.profiles.iter_mut().find(|p| p.id == active).and_then(selector) {
            Some(field) => {
                *field = value;
                true
            }
            None => false,
        }
    }

    /// A whole number, clamped into its range. A value that is not a number is refused whole: the
    /// row keeps what it had and the notice line says what happened.
    fn set_number(&mut self, min: u32, max: u32, value: String) -> bool {
        match value.trim().parse::<u32>() {
            Ok(number) => {
                self.listening.silence_ms = number.clamp(min, max).to_string();
                true
            }
            Err(_) => {
                self.refuse();
                false
            }
        }
    }

    fn set_gain(&mut self, value: String) -> bool {
        match value.trim().parse::<f32>() {
            Ok(number) => {
                self.listening.mic_gain = number.clamp(MIC_GAIN_MIN, MIC_GAIN_MAX).to_string();
                true
            }
            Err(_) => {
                self.refuse();
                false
            }
        }
    }

    fn refuse(&mut self) {
        self.notice = Some((NoticeKind::Warning, strings(UiLanguage::Es).cfg_value_not_whole.into()));
    }

    /// Moves to the next or previous tab, wrapping, and drops nothing silently: rows are per tab,
    /// so the focus starts over at the first one.
    fn switch_tab(&mut self, step: i8) {
        let tabs = [ConfigTab::Providers, ConfigTab::Listening, ConfigTab::Profiles, ConfigTab::Server];
        let at = tabs.iter().position(|tab| *tab == self.tab).unwrap_or(0);
        let len = tabs.len();
        let next = if step >= 0 { (at + 1) % len } else { (at + len - 1) % len };
        self.tab = tabs[next];
        self.field = 0;
    }

    fn next_field(&mut self) {
        let last = self.field_rows().len().saturating_sub(1);
        if self.field < last {
            self.field += 1;
        }
    }

    fn prev_field(&mut self) {
        self.field = self.field.saturating_sub(1);
    }

    /// Selects a profile. An index the current list no longer holds is a no-op: a stale click is
    /// not a panic, and it is not a silent switch either.
    pub fn select_profile(&mut self, index: usize) {
        if let Some(profile) = self.profiles.get(index) {
            self.active_profile = profile.id.clone();
            self.dirty = true;
        }
    }

    /// The profile the form is editing, if the list still holds it.
    fn active(&self) -> Option<&ProfileForm> {
        self.profiles.iter().find(|p| p.id == self.active_profile)
    }

    /// Which capability group the focused Providers row belongs to.
    fn provider_group(&self) -> Capability {
        self.providers.get(self.field / PROVIDER_ROW_COUNT).map(|p| p.capability).unwrap_or(Capability::Chat)
    }

    /// True when the focused Providers row is its group's Test button: the last row of the group.
    fn is_test_row(&self) -> bool {
        self.field % PROVIDER_ROW_COUNT == PROVIDER_ROW_COUNT - 1
    }

    /// The tab's rows, which is also what `field` indexes.
    fn field_rows(&self) -> Vec<FormRow> {
        self.rows()
    }

    fn provider_rows(&self) -> Vec<FormRow> {
        let es = strings(UiLanguage::Es);
        let mut rows = Vec::new();
        for provider in &self.providers {
            rows.push(FormRow {
                label: es.cfg_connector,
                value: provider.kind.clone(),
                editable: false,
                is_active: false,
            });
            rows.push(FormRow {
                label: es.cfg_base_url,
                value: provider.base_url.clone(),
                editable: true,
                is_active: false,
            });
            rows.push(FormRow {
                label: es.cfg_model,
                value: provider.model.clone(),
                editable: true,
                is_active: false,
            });
            rows.push(FormRow {
                label: es.cfg_key_name,
                value: provider.key_name.clone(),
                editable: true,
                is_active: false,
            });
            rows.push(FormRow {
                label: es.cfg_test,
                value: String::new(),
                editable: false,
                is_active: false,
            });
        }
        rows
    }

    fn listening_rows(&self) -> Vec<FormRow> {
        let es = strings(UiLanguage::Es);
        let mode = match self.listening.mode {
            ListeningMode::Push => es.mode_push,
            ListeningMode::Auto => es.mode_auto,
            ListeningMode::HandsFree => es.mode_hands_free,
        };
        let mut rows = vec![
            FormRow { label: es.cfg_mode, value: mode.to_owned(), editable: false, is_active: false },
            FormRow {
                label: es.cfg_silence_ms,
                value: self.listening.silence_ms.clone(),
                editable: true,
                is_active: false,
            },
            FormRow {
                label: es.cfg_mic_gain,
                value: self.listening.mic_gain.clone(),
                editable: true,
                is_active: false,
            },
        ];
        // An absent device is an absent row, not an empty one; a named device with an empty name is
        // still a choice, so it keeps its row.
        if let Some(device) = &self.listening.mic_device {
            rows.push(FormRow {
                label: es.cfg_mic_device,
                value: device.clone(),
                editable: true,
                is_active: false,
            });
        }
        rows
    }

    fn profile_rows(&self) -> Vec<FormRow> {
        let es = strings(UiLanguage::Es);
        let mut rows = Vec::new();
        // Every profile the form holds, one row each: the list is what `SelectProfile` indexes.
        for profile in &self.profiles {
            let active = profile.id == self.active_profile;
            // The active profile is marked in the row itself: a reader sees which profile the
            // assistant is on without a second marker column.
            let value = if active { format!("◉ {}", profile.name) } else { profile.name.clone() };
            rows.push(FormRow {
                label: es.cfg_profile,
                value,
                editable: false,
                is_active: active,
            });
        }
        // The field rows belong to the active profile, in this order (the id last). This order is
        // what `commit_profile` maps a row back to, and the two live side by side so a reorder of
        // one is a visible edit of the other.
        if let Some(profile) = self.active() {
            for (label, value) in [
                (es.cfg_name, &profile.name),
                (es.cfg_language, &profile.language),
                (es.cfg_prompt, &profile.prompt),
                (es.cfg_voice_instructions, &profile.voice_instructions),
                (es.cfg_greeting, &profile.greeting),
                (es.cfg_wake_word, &profile.wake_word),
                (es.cfg_profile, &profile.id),
            ] {
                rows.push(FormRow { label, value: value.clone(), editable: true, is_active: false });
            }
            // The voices table is last and stays inside its bounds: two rows per entry, the
            // connector (a list) and the voice name (editable).
            for (connector, voice) in &profile.voices {
                rows.push(FormRow {
                    label: es.cfg_voices,
                    value: connector.clone(),
                    editable: false,
                    is_active: false,
                });
                rows.push(FormRow {
                    label: es.cfg_voices,
                    value: voice.clone(),
                    editable: true,
                    is_active: false,
                });
            }
        }
        rows
    }

    /// The Server tab: the bind address, the LAN switch, the token, the row that mints a new one,
    /// and Save. The token is drawn like any other value, so an unset token reads as the unset word
    /// and a set one shows the token the file would hold; the file changes on Save, never here.
    fn server_rows(&self) -> Vec<FormRow> {
        let es = strings(UiLanguage::Es);
        vec![
            FormRow {
                label: es.cfg_bind,
                value: self.server.bind.clone(),
                editable: true,
                is_active: false,
            },
            FormRow {
                label: es.cfg_lan,
                value: if self.server.lan { es.cfg_on } else { es.cfg_off }.to_owned(),
                editable: false,
                is_active: false,
            },
            FormRow {
                label: es.cfg_token,
                value: if self.unset() { es.cfg_unset.to_owned() } else { self.server.token.clone() },
                editable: false,
                is_active: false,
            },
            FormRow {
                label: es.cfg_regenerate,
                value: String::new(),
                editable: false,
                is_active: false,
            },
            FormRow {
                label: es.cfg_save,
                value: String::new(),
                editable: false,
                is_active: false,
            },
        ]
    }

    /// Shows the answer to a `Test` request in the screen's notice line: green when the provider
    /// answered, red when it refused.
    ///
    /// The text is the capability's own name and what the probe said; the form adds no copy of its
    /// own, because it does not know which of the two string tables is in use (the plan's
    /// `ConfigForm` has no language field). The latency is shown only when there was a call to
    /// measure: a refusal that never left the machine — a missing key — carries no time at all.
    pub fn test_result(&mut self, capability: Capability, ok: bool, latency_ms: u64, message: String) {
        let text = if ok {
            format!("{capability}: {latency_ms} ms — {message}")
        } else {
            format!("{capability}: {message}")
        };
        self.notice = Some((if ok { NoticeKind::Ok } else { NoticeKind::Error }, text));
    }
}

/// The rows a Providers capability group has: connector, base URL, model, key name and its Test
/// button. `field / PROVIDER_ROW_COUNT` is the group, `field % PROVIDER_ROW_COUNT` the row in it.
const PROVIDER_ROW_COUNT: usize = 5;

/// The active profile's voice entries, in the sorted order `Config`'s `BTreeMap` arrives in.
fn active_voices(form: &ConfigForm) -> &[(String, String)] {
    form.active().map(|profile| profile.voices.as_slice()).unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strings::{strings, EN, ES};

    /// A form open on Providers: what `ConfigAction::Open` must produce, spelled out. It is filled,
    /// because a form that a client converted from a file is the only one that can be saved.
    fn providers() -> ConfigForm {
        let mut f = ConfigForm::default();
        f.mark_filled();
        f.providers = vec![
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
        ];
        f
    }

    /// A form open on Listening, with the mode cycling test's start state explicit.
    fn listening() -> ConfigForm {
        let mut f = providers();
        f.tab = ConfigTab::Listening;
        f
    }

    /// Two profiles, the second one active, with one voice each.
    fn profiles() -> ConfigForm {
        let mut f = providers();
        f.tab = ConfigTab::Profiles;
        f.active_profile = "meigo".into();
        f.profiles = vec![
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
        f
    }

    /// The effects the screen asked the client for, with the no-ops dropped: the twelve tests below
    /// read a short list instead of eleven empty `Vec`s.
    fn effects(effects: Vec<ConfigEffect>) -> Vec<ConfigEffect> {
        effects.into_iter().filter(|e| *e != ConfigEffect::None).collect()
    }

    /// Types `text` one character at a time, the way a terminal delivers it.
    fn type_text(f: &mut ConfigForm, text: &str) {
        for c in text.chars() {
            f.apply(ConfigAction::Push(c));
        }
    }

    /// Moves focus to the row with this label, inside the current tab, and stops there: a label is
    /// what a reader knows, `field` is what the code moves.
    fn focus(f: &mut ConfigForm, label: &str) {
        let at = f.row_labels().iter().position(|l| *l == label).unwrap_or_else(|| panic!("no {label:?} row"));
        f.field = at;
    }

    /// The active profile's scalar fields, in the order `profile_rows` emits them: name, language,
    /// prompt, voice instructions, greeting, wake word, id. Spelled out here as the test's own
    /// expectation, deliberately not read from the renderer.
    fn active_fields(f: &ConfigForm) -> [String; PROFILE_FIELD_COUNT] {
        let p = f.active().expect("an active profile");
        [
            p.name.clone(),
            p.language.clone(),
            p.prompt.clone(),
            p.voice_instructions.clone(),
            p.greeting.clone(),
            p.wake_word.clone(),
            p.id.clone(),
        ]
    }

    #[test]
    fn the_tabs_wrap_around_in_both_directions() {
        let mut f = ConfigForm::default();
        assert_eq!(f.tab, ConfigTab::Providers);
        assert!(effects(f.apply(ConfigAction::PrevTab)).is_empty());
        assert_eq!(f.tab, ConfigTab::Server, "backwards from the first tab wraps");
        assert_eq!(f.tab_title(f.tab, strings(UiLanguage::Es)), ES.cfg_server);

        f.tab = ConfigTab::Server;
        f.field = 2;
        f.apply(ConfigAction::NextTab);
        assert_eq!(f.tab, ConfigTab::Providers, "forwards from the last tab wraps");
        assert_eq!(f.tab_title(f.tab, strings(UiLanguage::En)), EN.cfg_providers);
        assert_eq!(f.field, 0, "the rows belong to the tab, so the focus starts over");
    }

    #[test]
    fn the_field_stops_at_the_first_and_last_row_of_its_tab() {
        let mut f = providers();
        f.field = 0;
        f.apply(ConfigAction::PrevField);
        assert_eq!(f.field, 0, "there is no row before the first one");

        let last = f.rows().len() - 1;
        f.field = last;
        assert_eq!(f.rows()[last].label, ES.cfg_test, "the last provider row is the third group's Test row");
        f.apply(ConfigAction::NextField);
        assert_eq!(f.field, last, "and there is none after the last one");

        f.field = 0;
        f.apply(ConfigAction::NextField);
        assert_eq!(f.field, 1, "inside the list it still moves");

        // An empty row list cannot be walked into a panic: this is what a form with no profile
        // would look like, and `field` has nowhere to go.
        let mut empty = ConfigForm::default();
        empty.profiles.clear();
        empty.tab = ConfigTab::Profiles;
        empty.field = 0;
        empty.apply(ConfigAction::NextField);
        assert_eq!(empty.field, 0);
        assert!(empty.rows().is_empty());
    }

    #[test]
    fn the_providers_tab_shows_one_group_of_rows_for_each_capability() {
        let f = providers();
        let rows = f.rows();
        assert_eq!(rows.len(), 15, "five rows per capability, three capabilities");
        assert_eq!(rows.len(), f.row_labels().len(), "rows and labels are the same rows");
        assert_eq!(rows[0].label, ES.cfg_connector);
        assert_eq!(rows[0].value, "openai_compat");
        assert!(!rows[0].editable, "the connector is a list, not a text row");
        assert_eq!(rows[1].label, ES.cfg_base_url);
        assert!(rows[1].editable);
        assert_eq!(rows[1].value, "https://openrouter.ai/api/v1");
        assert_eq!(rows[4].label, ES.cfg_test, "every group ends with its own Test row");
        assert!(!rows[4].editable);
        assert_eq!(rows[5].label, ES.cfg_connector, "the second capability starts over");
        assert_eq!(rows[5].value, "openai_compat");
        assert_eq!(rows[9].label, ES.cfg_test, "and so does its group");
        assert_eq!(rows[10].label, ES.cfg_connector, "and the third one");
        assert_eq!(rows[14].label, ES.cfg_test);
        assert_eq!(rows[3].label, ES.cfg_key_name);
        assert_eq!(rows[3].value, "openrouter", "the key name is what the row shows, never a key");
    }

    #[test]
    fn the_listening_tab_hides_the_device_row_when_no_device_is_configured() {
        let mut f = listening();
        assert_eq!(f.rows().len(), 3, "the absent device is an absent row, not an empty one");
        let rows = f.rows();
        assert_eq!(rows[0].label, ES.cfg_mode);
        assert!(!rows[0].editable, "the mode is cycled with the toggle key");
        assert_eq!(rows[0].value, ES.mode_push);
        assert_eq!(rows[1].label, ES.cfg_silence_ms);
        assert!(rows[1].editable);
        assert_eq!(rows[1].value, "1000");
        assert_eq!(rows[2].label, ES.cfg_mic_gain);
        assert_eq!(rows[2].value, "12.0");

        f.listening.mic_device = Some("Rode NT-USB".into());
        let rows = f.rows();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[3].label, ES.cfg_mic_device);
        assert_eq!(rows[3].value, "Rode NT-USB");
        assert!(rows[3].editable);
        f.listening.mic_device = Some(String::new());
        assert_eq!(f.rows().len(), 4, "an empty name is a named device: the row stays on screen");
    }

    #[test]
    fn the_profiles_tab_shows_the_active_marker_before_the_fields_and_the_voices_last() {
        let mut f = profiles();
        // Two profile-list rows, the active profile's seven field rows and its one voice's two rows.
        let count = ConfigForm::profile_row_count(&f);
        assert_eq!(count, 11);
        assert_eq!(f.rows().len(), count);
        let rows = f.rows();
        assert_eq!(rows[0].label, ES.cfg_profile);
        assert_eq!(rows[0].value, "El Compae", "the list shows the names, one row per profile");
        assert!(!rows[0].is_active);
        assert!(rows[1].value.contains("O Meigo"));
        assert!(rows[1].value.contains("◉"), "the active profile is marked in the row itself");
        assert!(rows[1].is_active);
        assert_eq!(rows[2].label, ES.cfg_name, "the fields are the same for every profile");
        assert_eq!(rows[2].value, "O Meigo");
        let voices_at = f.voice_label_row();
        assert_eq!(voices_at, 9, "two list rows and seven field rows come first");
        assert_eq!(rows[voices_at].label, ES.cfg_voices, "the voice rows are last");
        assert_eq!(rows[voices_at].value, "gemini");
        assert_eq!(rows[rows.len() - 1].value, "Puck", "the voices shown are the active profile's");

        f.active_profile = "compae".into();
        let rows = f.rows();
        assert!(rows[0].is_active);
        assert!(!rows[1].is_active);
        assert_eq!(rows[rows.len() - 1].value, "Charon");
    }

    #[test]
    fn the_server_tab_ends_with_the_save_row_and_shows_the_unset_token() {
        let mut f = providers();
        f.tab = ConfigTab::Server;
        assert!(f.unset(), "an empty token is the unset state");
        let rows = f.rows();
        assert_eq!(rows.len(), 5, "bind, lan, token, regenerate and Save");
        assert_eq!(rows[0].label, ES.cfg_bind);
        assert_eq!(rows[0].value, "127.0.0.1:7070");
        assert_eq!(rows[1].label, ES.cfg_lan);
        assert!(!rows[1].editable);
        assert_eq!(rows[1].value, ES.cfg_off, "a false LAN switch is off, not unset");
        assert_eq!(rows[2].label, ES.cfg_token);
        assert_eq!(rows[2].value, ES.cfg_unset, "an empty token is shown as the unset word");
        assert!(!rows[2].editable, "the token is minted, never typed");
        assert_eq!(rows[3].label, ES.cfg_regenerate, "and the row that mints it is beside it");
        assert_eq!(rows[4].label, ES.cfg_save, "the Save row is the tab's last row");
        assert!(!rows[4].is_active, "nothing is toggled on: Save is an action, not a switch");
        assert!(!rows[4].editable, "and it is not a text row either");

        f.server.lan = true;
        assert_eq!(f.rows()[1].value, ES.cfg_on);
        f.server.token = "0123456789abcdef0123456789abcdef".into();
        assert!(!f.unset());
        assert_eq!(f.rows()[2].value, "0123456789abcdef0123456789abcdef", "the token is on screen");
    }

    #[test]
    fn an_edit_starts_empty_and_replaces_the_value_on_commit() {
        let mut f = providers();
        focus(&mut f, ES.cfg_model);
        assert!(effects(f.apply(ConfigAction::BeginEdit)).is_empty());
        assert_eq!(f.editing.as_deref(), Some(""), "the buffer replaces the value, it does not extend it");
        assert_eq!(f.rows()[f.field].value, "google/gemini-2.5-flash", "the field still reads what it had");
        assert!(!f.dirty);

        type_text(&mut f, "ollama/llama3");
        assert_eq!(f.editing.as_deref(), Some("ollama/llama3"));
        assert!(effects(f.apply(ConfigAction::CommitEdit)).is_empty());
        assert_eq!(f.editing, None);
        assert_eq!(f.providers[0].model, "ollama/llama3");
        assert!(f.dirty, "a committed edit is an unsaved change");
        assert_eq!(f.rows()[f.field].value, "ollama/llama3");
    }

    #[test]
    fn backspace_removes_the_last_character_of_the_buffer_and_nothing_else() {
        let mut f = providers();
        focus(&mut f, ES.cfg_key_name);
        f.apply(ConfigAction::BeginEdit);
        type_text(&mut f, "openroutr");
        f.apply(ConfigAction::Backspace);
        assert_eq!(f.editing.as_deref(), Some("openrout"));
        f.apply(ConfigAction::CommitEdit);
        assert_eq!(f.providers[0].key_name, "openrout");

        // An empty buffer is a state, not a panic: the value is replaced by nothing.
        f.apply(ConfigAction::BeginEdit);
        f.apply(ConfigAction::Backspace);
        assert_eq!(f.editing.as_deref(), Some(""));
        f.apply(ConfigAction::CommitEdit);
        assert_eq!(f.providers[0].key_name, "");
        assert!(f.dirty);
    }

    #[test]
    fn a_number_that_is_no_number_is_refused_and_says_so() {
        let mut f = listening();
        focus(&mut f, ES.cfg_silence_ms);
        f.apply(ConfigAction::BeginEdit);
        type_text(&mut f, "mil");
        f.apply(ConfigAction::CommitEdit);
        assert_eq!(f.listening.silence_ms, "1000", "the row keeps the value it had");
        assert!(!f.dirty, "a refused edit is not a change to save");
        assert_eq!(
            f.notice.as_ref().map(|(_, text)| text.as_str()),
            Some(ES.cfg_value_not_whole),
            "the screen says what happened"
        );

        // A number that parses is clamped into its range rather than refused.
        focus(&mut f, ES.cfg_silence_ms);
        f.apply(ConfigAction::BeginEdit);
        type_text(&mut f, "4");
        f.apply(ConfigAction::CommitEdit);
        assert_eq!(f.listening.silence_ms, "100");

        f.apply(ConfigAction::BeginEdit);
        type_text(&mut f, "999999");
        f.apply(ConfigAction::CommitEdit);
        assert_eq!(f.listening.silence_ms, "30000");

        focus(&mut f, ES.cfg_mic_gain);
        f.apply(ConfigAction::BeginEdit);
        type_text(&mut f, "7.5");
        f.apply(ConfigAction::CommitEdit);
        assert_eq!(f.listening.mic_gain, "7.5");

        f.apply(ConfigAction::BeginEdit);
        type_text(&mut f, "99");
        f.apply(ConfigAction::CommitEdit);
        assert_eq!(f.listening.mic_gain, "30");
        assert!(f.dirty);
    }

    #[test]
    fn clearing_the_device_row_means_the_system_default_device() {
        let mut f = listening();
        f.listening.mic_device = Some("Rode NT-USB".into());
        focus(&mut f, ES.cfg_mic_device);
        f.apply(ConfigAction::BeginEdit);
        f.apply(ConfigAction::CommitEdit);
        assert_eq!(f.listening.mic_device, None, "no device chosen is the absent row");
        assert_eq!(f.rows().len(), 3);
        assert!(f.dirty);
    }

    #[test]
    fn the_toggle_cycles_the_listening_mode_and_the_server_toggle() {
        let mut f = listening();
        f.field = 0;
        assert_eq!(f.listening.mode, ListeningMode::Push);
        for (expected, wire) in [
            (ListeningMode::Auto, ES.mode_auto),
            (ListeningMode::HandsFree, ES.mode_hands_free),
            (ListeningMode::Push, ES.mode_push),
        ] {
            assert!(effects(f.apply(ConfigAction::Toggle)).is_empty());
            assert_eq!(f.listening.mode, expected);
            assert_eq!(f.rows()[0].value, wire);
            assert!(f.dirty);
        }
        // The mode has exactly three states, so the fourth toggle is where it started.
        f.apply(ConfigAction::Toggle);
        assert_eq!(f.listening.mode, ListeningMode::Auto);

        f.tab = ConfigTab::Server;
        f.field = 1;
        assert!(!f.server.lan);
        assert!(effects(f.apply(ConfigAction::Toggle)).is_empty());
        assert!(f.server.lan);
        f.apply(ConfigAction::Toggle);
        assert!(!f.server.lan);

        // A row that is not a toggle is left exactly as it was.
        f.tab = ConfigTab::Listening;
        f.field = 1;
        let before = f.listening.silence_ms.clone();
        f.apply(ConfigAction::Toggle);
        assert_eq!(f.listening.silence_ms, before);
    }

    #[test]
    fn choosing_a_profile_sets_the_active_id_and_leaves_the_row_focus_alone() {
        let mut f = profiles();
        f.field = 3;
        assert!(effects(f.apply(ConfigAction::SelectProfile(0))).is_empty());
        assert_eq!(f.active_profile, "compae");
        assert_eq!(f.field, 3, "clicking a profile chooses it; it does not move the keyboard focus");
        assert!(f.dirty);

        // A stale click cannot index past the list, and cannot panic either.
        f.apply(ConfigAction::SelectProfile(9));
        assert_eq!(f.active_profile, "compae", "out of range is a no-op");
    }

    /// The point of finding A: every one of the seven scalar profile rows writes *its own* field.
    /// Each row is focused by its index — the id row and the list rows share a label, so a label
    /// cannot name them — a distinct value is typed and committed, and the assertion is on which
    /// field the value landed in and that nothing else moved.
    #[test]
    fn each_profile_row_commits_its_own_field() {
        for (offset, attribute, written) in [
            (0usize, "name", "written-name"),
            (1, "language", "en-GB"),
            (2, "prompt", "written-prompt"),
            (3, "voice_instructions", "written-voice"),
            (4, "greeting", "written-greeting"),
            (5, "wake_word", "written-wake"),
            (6, "id", "written-id"),
        ] {
            let mut f = profiles();
            // `meigo` is the active profile, so its field rows start just past the two list rows.
            let row = f.profiles.len() + offset;
            assert_eq!(
                f.rows()[row].value,
                active_fields(&f)[offset],
                "{attribute}: the row shows its own value"
            );
            let before = active_fields(&f);

            f.field = row;
            f.apply(ConfigAction::BeginEdit);
            type_text(&mut f, written);
            f.apply(ConfigAction::CommitEdit);

            let after = active_fields(&f);
            assert_eq!(
                after[offset], written,
                "{attribute}: the value landed in the wrong field: {after:?}"
            );
            if offset == 6 {
                // The id is the one field the active-profile pointer follows, or a rename would
                // switch the assistant to a profile that no longer exists.
                assert_eq!(f.active_profile, written, "{attribute}: the renamed profile stays active");
                continue;
            }
            assert_eq!(after[6], before[6], "{attribute}: the id must not move");
            assert_eq!(
                f.active_profile, before[6],
                "{attribute}: no other profile may become active"
            );
            for (index, expected) in before.iter().enumerate() {
                if index == offset {
                    // The field the value was written into is the one that must have changed; every
                    // other one must not.
                    continue;
                }
                assert_eq!(&after[index], expected, "{attribute}: field {index} must not change");
            }
        }
    }

    /// The keyboard and the mouse reach the same tabs, and neither of them is `F9`: that key is the
    /// door in and out of the screen, so no tab may answer it.
    #[test]
    fn the_tab_keys_select_tabs_and_never_f9() {
        let mut f = providers();
        for (key, tab) in [
            (5u8, ConfigTab::Providers),
            (6, ConfigTab::Listening),
            (8, ConfigTab::Profiles),
            (10, ConfigTab::Server),
        ] {
            f.tab = ConfigTab::Providers;
            f.field = 3;
            assert_eq!(f.key(KeyInput::F(key)).action, Some(ConfigAction::SelectTab(tab)));
            assert_eq!(f.tab, tab, "F{key} selects {tab:?}");
            assert_eq!(f.field, 0, "and the rows belong to the tab, so the focus starts over");
        }
        assert_eq!(f.key(KeyInput::F(9)).action, None, "F9 is the screen's door, not a tab");
        assert_eq!(f.key(KeyInput::F(2)).action, None, "F2 is only the Providers tab's Test");
        assert_eq!(f.key(KeyInput::F(4)).action, Some(ConfigAction::Save), "F4 is the Save button");
    }

    /// The click path: a control names the target it is drawn on, so a click cannot actuate
    /// whichever row happened to be focused.
    #[test]
    fn a_clicked_row_is_focused_and_actuated() {
        let mut f = providers();
        f.field = 0;
        // A text row: the click focuses it and opens its buffer, exactly as `Enter` would.
        assert!(effects(f.apply(ConfigAction::FocusRow(2))).is_empty());
        assert_eq!(f.field, 2, "the clicked row is the focused one");
        assert_eq!(f.editing.as_deref(), Some(""), "and a text row opens its buffer");
        f.apply(ConfigAction::Close);

        // A button row: the click focuses it and presses it.
        assert_eq!(
            effects(f.apply(ConfigAction::FocusRow(4))),
            vec![ConfigEffect::Test(Capability::Chat)],
            "a click on the first group's Test row probes that group"
        );
        assert_eq!(f.field, 4);

        // A row the tab no longer has is a no-op, not a panic.
        f.apply(ConfigAction::FocusRow(99));
        assert_eq!(f.field, 4);
        assert!(f.editing.is_none());
    }

    /// A click on a profile-list row chooses that profile; it does not open a buffer and it does
    /// not run whichever row was focused (the only way to change profile from the screen).
    #[test]
    fn a_clicked_profile_row_becomes_the_active_profile() {
        let mut f = profiles();
        f.field = 3;
        assert!(effects(f.apply(ConfigAction::FocusRow(0))).is_empty());
        assert_eq!(f.active_profile, "compae", "the clicked row is the profile that becomes active");
        assert_eq!(f.field, 0, "and the focus follows the click");
        assert!(f.editing.is_none(), "a profile is chosen, not edited");
        assert!(f.dirty);
    }

    #[test]
    fn save_and_regenerate_are_the_only_effects_the_reducer_raises() {
        let mut f = providers();
        assert_eq!(effects(f.apply(ConfigAction::Save)), vec![ConfigEffect::Save]);
        assert!(f.dirty);

        // Every capability has a Test row, and the row names the connector to probe.
        for (field, capability) in
            [(1, Capability::Chat), (6, Capability::Speech), (11, Capability::Transcribe)]
        {
            f.tab = ConfigTab::Providers;
            f.field = field;
            assert_eq!(
                effects(f.apply(ConfigAction::Test)),
                vec![ConfigEffect::Test(capability)],
                "the group the focused row belongs to decides which connector is probed"
            );
        }
        // And the Test row of a group is the same button: activating it raises the same effect.
        f.field = 14;
        assert_eq!(effects(f.apply(ConfigAction::Toggle)), vec![ConfigEffect::Test(Capability::Transcribe)]);

        // On the Server tab the token is regenerated from its own row and the form is left
        // untouched: the client mints the value (Task 6) and writes it into this row.
        f.tab = ConfigTab::Server;
        f.field = SERVER_ROW_REGENERATE;
        assert_eq!(effects(f.apply(ConfigAction::RegenerateToken)), vec![ConfigEffect::RegenerateToken]);
        assert!(f.dirty);
        assert_eq!(
            effects(f.apply(ConfigAction::FocusRow(SERVER_ROW_REGENERATE))),
            vec![ConfigEffect::RegenerateToken],
            "and the row the screen draws raises it too"
        );
        // And the row is an action rather than a switch: activating it twice does not flip anything.
        assert!(!f.server.lan);
    }

    /// The placeholder the screen opens on is not the user's configuration, so it is not saveable:
    /// a `config.toml` read that failed must not be overwritten by blank base URLs and models.
    #[test]
    fn an_unfilled_form_refuses_to_save_and_a_filled_one_saves() {
        let mut placeholder = ConfigForm::default();
        assert!(!placeholder.filled(), "a placeholder is not filled");
        assert!(effects(placeholder.apply(ConfigAction::Save)).is_empty(), "and it refuses to save");
        assert!(!placeholder.dirty, "a refused save is not an unsaved change");
        assert_eq!(
            placeholder.key(KeyInput::Char(SAVE_KEY)).effects,
            Vec::new(),
            "Ctrl+S is refused the same way"
        );

        placeholder.mark_filled();
        assert!(placeholder.filled());
        assert_eq!(effects(placeholder.apply(ConfigAction::Save)), vec![ConfigEffect::Save]);
        assert!(placeholder.dirty);
    }

    /// The Server tab draws its own Save row, and that row is a second door onto a write: it has to
    /// refuse an unfilled form exactly as the action does, or a click would bypass the guard.
    #[test]
    fn the_server_tab_cannot_save_an_unfilled_form() {
        // Built in one expression rather than assigned into a `ConfigForm::default()`: the branch's
        // clippy run flags `field_reassign_with_default` on the two-line shape.
        let mut f = ConfigForm { tab: ConfigTab::Server, ..ConfigForm::default() };
        assert!(!f.filled(), "the placeholder is not filled");
        assert!(f.apply(ConfigAction::FocusRow(SERVER_ROW_SAVE)).is_empty());
        assert!(!f.dirty, "and the Save row refuses it too");

        f.mark_filled();
        assert_eq!(f.apply(ConfigAction::FocusRow(SERVER_ROW_SAVE)), vec![ConfigEffect::Save]);
        assert!(f.dirty);
    }

    #[test]
    fn ctrl_r_regenerates_the_token_without_editing_any_row() {
        let mut f = providers();
        f.tab = ConfigTab::Server;
        // The token row is the first row of the tab, and the shortcut works from anywhere in it.
        f.field = 0;
        let outcome = f.key(KeyInput::Char('\u{12}'));
        assert_eq!(outcome.action, Some(ConfigAction::RegenerateToken));
        assert_eq!(outcome.effects, vec![ConfigEffect::RegenerateToken]);
        assert!(f.editing.is_none(), "a shortcut is not an edit");
        assert_eq!(f.server.bind, "127.0.0.1:7070", "and it writes no row itself: the client mints the token");
    }

    #[test]
    fn closing_discards_the_edits_and_leaves_the_form_as_it_was() {
        let mut f = providers();
        focus(&mut f, ES.cfg_base_url);
        f.apply(ConfigAction::BeginEdit);
        type_text(&mut f, "http://localhost:11434/v1");
        assert!(f.editing.is_some());
        f.apply(ConfigAction::Close);
        assert_eq!(f.editing, None, "the half-typed value is the edit, and it goes with the screen");
        assert_eq!(f.providers[0].base_url, "https://openrouter.ai/api/v1", "the field never moved");
        assert!(!f.dirty, "nothing was written, so there is nothing unsaved either");
    }

    #[test]
    fn the_navigation_keys_mean_the_same_thing_as_the_actions() {
        let mut f = providers();
        assert_eq!(f.key(KeyInput::F(9)).action, None, "the screen is already open");
        assert_eq!(f.key(KeyInput::Down).action, Some(ConfigAction::NextField));
        assert_eq!(f.key(KeyInput::Up).action, Some(ConfigAction::PrevField));
        assert_eq!(f.key(KeyInput::Tab).action, Some(ConfigAction::NextTab));
        assert_eq!(f.key(KeyInput::Right).action, Some(ConfigAction::NextTab));
        assert_eq!(f.key(KeyInput::Left).action, Some(ConfigAction::PrevTab));
        assert_eq!(f.key(KeyInput::Esc).action, Some(ConfigAction::Close));
        assert_eq!(f.key(KeyInput::Char('\u{13}')).action, Some(ConfigAction::Save));
        assert_eq!(f.key(KeyInput::Char('\u{12}')).action, Some(ConfigAction::RegenerateToken));
        assert_eq!(f.key(KeyInput::Char('\u{13}')).effects, vec![ConfigEffect::Save]);
        assert_eq!(f.key(KeyInput::Char('\u{12}')).effects, vec![ConfigEffect::RegenerateToken]);
        // A printable key outside an edit is not a text entry: it types nothing into the void, and
        // the typing case is the next test's, where a buffer is open. A space is not a second
        // `Enter` either — the copy no longer promises one.
        assert_eq!(f.key(KeyInput::Char('x')).action, None);
        assert_eq!(f.key(KeyInput::Char('x')).effects, Vec::new());
        assert_eq!(f.key(KeyInput::Char(' ')).action, None);
        assert_eq!(f.key(KeyInput::Char(' ')).effects, Vec::new());

        f.field = 0;
        assert_eq!(f.key(KeyInput::Enter).action, Some(ConfigAction::Toggle), "a toggle row has no text to edit");
        // The row below it is text, and that one opens a buffer instead: `Enter` means the row's
        // own action, and only an editable row has a buffer to open.
        f.field = 1;
        assert_eq!(f.key(KeyInput::Enter).action, Some(ConfigAction::BeginEdit));
    }

    #[test]
    fn while_a_row_is_being_edited_the_keys_edit_and_do_not_navigate() {
        let mut f = providers();
        focus(&mut f, ES.cfg_model);
        f.apply(ConfigAction::BeginEdit);
        assert_eq!(f.key(KeyInput::Char('a')).action, Some(ConfigAction::Push('a')));
        assert_eq!(f.key(KeyInput::Backspace).action, Some(ConfigAction::Backspace));
        // The two navigation keys are checked while the buffer is still open, because committing is
        // what closes it: `Down` is neither text nor a way out, and `Tab` may not jump tabs mid-edit.
        assert_eq!(
            f.key(KeyInput::Down).action,
            None,
            "Down is neither text nor a way out: it must not move the focus mid-edit"
        );
        assert_eq!(f.key(KeyInput::Tab).action, None, "nor may Tab jump to the next tab");
        // A click while the buffer is open is not a way out of it either: the row under the pointer
        // does not move the focus and does not press anything behind the edit.
        assert_eq!(f.key(KeyInput::F(2)).action, None, "not even the Test key");
        assert!(effects(f.apply(ConfigAction::FocusRow(0))).is_empty());
        assert_eq!(f.field, 2, "the focus stayed where the buffer is: the model row of the first group");
        assert_eq!(f.editing.as_deref(), Some(""), "and the buffer is still open");
        assert_eq!(f.key(KeyInput::Enter).action, Some(ConfigAction::CommitEdit));
        assert_eq!(f.key(KeyInput::Esc).action, Some(ConfigAction::Close), "Esc is the screen's, not the buffer's");
    }

    /// The screen's copy is in the tables and nowhere else, and both tables carry every key the
    /// screen can show — including the ones the current state happens to keep off screen.
    #[test]
    fn both_string_tables_carry_every_key_the_screen_uses() {
        // The whole form, which is the widest set of keys any state can ask for: two profiles with a
        // voice each, a device chosen, and a token generated.
        let mut f = profiles();
        f.listening.mic_device = Some("Rode NT-USB".into());
        f.server.token = "0123456789abcdef0123456789abcdef".into();
        f.notice = Some((NoticeKind::Warning, ES.cfg_value_not_whole.into()));

        let mut keys = f.string_keys();
        for tab in [ConfigTab::Providers, ConfigTab::Listening, ConfigTab::Profiles, ConfigTab::Server] {
            for lang in [UiLanguage::Es, UiLanguage::En] {
                keys.push(f.tab_title(tab, strings(lang)));
            }
        }
        keys.push(ES.cfg_value_not_whole);
        keys.push(EN.cfg_value_not_whole);

        for key in keys {
            assert!(!key.trim().is_empty(), "a label may not be blank");
            assert!(
                ES.cfg_help.contains("F9") && EN.cfg_help.contains("F9"),
                "the help line says how the screen is opened: {}",
                ES.cfg_help
            );
            assert!(!ES.cfg_help_editing.trim().is_empty());
            assert!(!EN.cfg_help_editing.trim().is_empty());
        }

        // The two tables are different tables, not one table read twice.
        assert_ne!(ES.cfg_providers, EN.cfg_providers);
        assert_ne!(ES.cfg_save, EN.cfg_save);
        assert_ne!(ES.cfg_off, EN.cfg_off);
        // The help line promises no toggle this form cannot do: a space is not a key it has.
        assert!(!ES.cfg_help.to_lowercase().contains("espacio"), "{}", ES.cfg_help);
        assert!(!EN.cfg_help.to_lowercase().contains("space"), "{}", EN.cfg_help);
        // The half that removal took with it is true and belongs there: `Enter` is the toggle key for
        // the rows that are switches — `the_navigation_keys_mean_the_same_thing_as_the_actions`
        // asserts that behaviour — beside being the edit key for the rows that are text. Both tables
        // name it again, and neither may name the space key.
        assert!(
            ES.cfg_help.to_lowercase().contains("alternar") && EN.cfg_help.to_lowercase().contains("toggle"),
            "the help line must name the toggle `Enter` really does: {} / {}",
            ES.cfg_help,
            EN.cfg_help
        );
        // And the two tables that name the discard carry it in both languages.
        assert!(!ES.cfg_cancel.trim().is_empty() && !EN.cfg_cancel.trim().is_empty());
        assert!(ES.cfg_help_editing.contains(ES.cfg_cancel), "{}", ES.cfg_help_editing);
        assert!(EN.cfg_help_editing.contains(EN.cfg_cancel), "{}", EN.cfg_help_editing);

        // And the labels the rows hand out are exactly the table's.
        f.tab = ConfigTab::Providers;
        for label in f.row_labels() {
            assert!(
                [ES.cfg_connector, ES.cfg_base_url, ES.cfg_model, ES.cfg_key_name, ES.cfg_test]
                    .contains(&label),
                "an unknown row label: {label:?}"
            );
        }
    }

    /// Review Focus 5, as the screen shows it: an answer is green and carries the measured latency;
    /// a refusal is red and carries the provider's own words, with no latency dressed up as one. The
    /// base form is the module's own `providers()`, which is a form nobody has touched.
    #[test]
    fn a_test_result_lands_in_the_notice_and_never_dresses_up_a_failure() {
        let mut f = providers();

        f.test_result(Capability::Chat, true, 412, "18 bytes".into());
        let (kind, text) = f.notice.clone().unwrap();
        assert_eq!(kind, NoticeKind::Ok);
        assert_eq!(text, "chat: 412 ms — 18 bytes");

        f.test_result(
            Capability::Chat,
            false,
            0,
            "no API key named `openrouter`: run `tipsy-compadre key set openrouter` or set TIPSY_KEY_OPENROUTER"
                .into(),
        );
        let (kind, text) = f.notice.clone().unwrap();
        assert_eq!(kind, NoticeKind::Error);
        assert_eq!(
            text,
            "chat: no API key named `openrouter`: run `tipsy-compadre key set openrouter` or set TIPSY_KEY_OPENROUTER",
            "a refusal carries the answer as it came, and no latency"
        );

        // The third capability reads as a person says it, which is the protocol's `Display`.
        f.test_result(Capability::Transcribe, false, 30, "HTTP 500: boom".into());
        let (kind, text) = f.notice.clone().unwrap();
        assert_eq!(kind, NoticeKind::Error);
        assert_eq!(text, "transcription: HTTP 500: boom");
    }
}
