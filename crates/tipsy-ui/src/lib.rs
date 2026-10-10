//! Backend-agnostic ratatui HUD for tipsy-compadre.

pub mod config_form;
pub mod render;
pub mod render_config;
pub mod state;
pub mod strings;

pub use config_form::*;
pub use render::*;
pub use state::*;
pub use strings::{strings, Strings};
