//! Backend-agnostic ratatui HUD for tipsy-assistant.

pub mod config_form;
pub mod policy;
pub mod render;
pub mod render_config;
pub mod render_login;
pub mod state;
pub mod strings;
pub mod tone;

pub use config_form::*;
pub use render::*;
pub use state::*;
pub use strings::{Strings, strings};
pub use tone::{first_sentence, prompt_with_floor, brief_with_floor, TONE_FLOOR_EN, TONE_FLOOR_ES, TONE_FLOOR_LABEL};
