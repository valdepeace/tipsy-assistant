//! tipsy-assistant: server actors and the native terminal client.

pub mod text;

pub mod audio;

pub mod brain;

pub mod client;

pub mod config;

pub mod keys;

pub mod logging;

pub mod login;

pub mod providers;

pub mod realtime;

pub mod server;

pub mod session;

pub use session::turn_profile;

pub mod vad;

pub mod sysmon;
