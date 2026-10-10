//! HUD drawing.

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::state::{KeyInput, UiState};

pub struct ClockView {
    pub time: String,
    pub weekday: u8,
    pub day: u8,
    pub month: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub area: Rect,
    pub key: KeyInput,
}

pub fn draw(_frame: &mut Frame, _state: &UiState, _clock: &ClockView, _title: &str) -> Vec<Hit> {
    Vec::new()
}
