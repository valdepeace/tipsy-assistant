//! Realtime voice: one duplex WebSocket session with OpenAI's Realtime API (spec §6, plan 2C).
//!
//! `socket` is the transport — connect, configure, read, keep alive, reopen, close. `events` (Task
//! 3) is the pure mapping from the wire's JSON to the actions the rest of the module performs. The
//! call façade that ties both to the session, the microphone and the `Player` is Task 4 and lands
//! in this file.

pub mod socket;
