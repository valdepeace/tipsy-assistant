//! The browser's half of playback: one `AudioBufferSourceNode` per chunk.
//!
//! MDN, verbatim: "An `AudioBufferSourceNode` can only be played once; after each call to `start()`,
//! you have to create a new node if you want to play the same sound again. Fortunately, these nodes are
//! very inexpensive to create" — so one node per sentence is the design and not a compromise, and
//! holding the live ones is what makes barge-in possible (the same page's "don't even bother to hold a
//! reference" is about nodes nobody has to stop).
//!
//! **What is here and what is not.** Every decision — where a chunk starts, which node is still live,
//! which one has finished, what a command does to the timeline — is [`crate::playback`]'s, tested on
//! this host. This file is the presenter and holds no branch a test could reach: it turns a
//! [`crate::playback::Scheduled`] into an `AudioBufferSourceNode` on a given `AudioContext`, and stops
//! or forgets nodes by the ids the core hands back.
//!
//! **No `#[cfg(target_arch = "wasm32")]` on this file, and that is a measured decision rather than an
//! oversight** — the plan's Files list asks for one. The whole crate's wasm half compiles for the host
//! target (`web-sys`'s items are `extern "C"` declarations and `wasm-bindgen` emits a panicking
//! fallback outside wasm), which is what lets `socket.rs`, `mic.rs` and this file share one shape; and
//! `mic.rs` now carries sixteen host tests, so a manifest that hid `web-sys` from the host build could
//! not compile at all. Gating this file alone would put `#[cfg]` on five call sites in `lib.rs` and buy
//! nothing. The ledger's induced failures record the measurement.
//!
//! **What only a browser can check, named rather than implied:** whether `createBuffer` accepts a rate
//! other than the context's and resamples on playback (MDN says a mono buffer on a 44.1 kHz context
//! "will be automatically *resampled*", which is what makes the announced rate safe to pass through);
//! whether `start(when)` honours a `when` a few tens of milliseconds ahead *sample-accurately*; whether
//! an abrupt `stop()` — no ramp, no `GainNode` here on purpose: the plan has none and neither does the
//! prior art's `silence()` — is audible as a click on a real device; and whether the context's clock
//! advances as `Playback`'s arithmetic assumes. No host test and no script on this machine can reach
//! any of them.

use std::cell::RefCell;
use std::collections::HashMap;

use wasm_bindgen::prelude::*;
use web_sys::{AudioBufferSourceNode, AudioContext};

use crate::playback;

thread_local! {
    /// The nodes still playing, by the id `playback::Playback` gave them. A map rather than the prior
    /// art's bare `Set` only because the ids are what `barge_in` and `finished` name; the shape is its
    /// shape (`pet/src/ui/pet.ts:26`). Thread-local is enough: wasm is single-threaded and `draw_web`'s
    /// callbacks all run on it.
    static NODES: RefCell<HashMap<u64, AudioBufferSourceNode>> = RefCell::new(HashMap::new());
}

/// Schedules one chunk. `scheduled.at` is never in the past — `Playback::chunk` places it at least a
/// cushion ahead — so the node is always scheduled for a future moment and `start(when)`'s
/// play-at-once-on-a-past-`when` behaviour is never what makes this audible.
pub fn play(ctx: &AudioContext, scheduled: &playback::Scheduled) -> Result<(), JsValue> {
    // `createBuffer(numOfChannels, length, sampleRate)`: `length` is "the size of the buffer in
    // sample-frames", and "All browsers must support sample rates in at least the range 8,000 Hz to
    // 96,000 Hz" (MDN `BaseAudioContext.createBuffer()`), so the announced rate is always accepted. A
    // buffer whose rate differs from the context's is resampled by the graph rather than by this file,
    // which is why nothing here converts the samples itself.
    let buffer = ctx.create_buffer(1, scheduled.samples.len() as u32, scheduled.rate as f32)?;
    // `copyToChannel(source, channelNumber, startInChannel)` takes "A `Float32Array` that the channel
    // data will be copied from" (MDN `AudioBuffer.copyToChannel()`). The conversion is `pcm.rs:24-26`'s:
    // `i16` full scale is 32768, not 32767.
    let floats: Vec<f32> = scheduled.samples.iter().map(|s| *s as f32 / 32768.0).collect();
    buffer.copy_to_channel(&floats, 0)?;
    let node = ctx.create_buffer_source()?;
    node.set_buffer(Some(&buffer));
    node.connect_with_audio_node(&ctx.destination())?;
    node.start_with_when(scheduled.at)?;
    NODES.with_borrow_mut(|nodes| nodes.insert(scheduled.id, node));
    Ok(())
}

/// Barge-in: the prior art's `silence()` (`pet/src/ui/pet.ts:33`) — `stop()` on every node that is
/// playing and forget them all. `stop()` with no argument or 0 "causes the sound to stop playback
/// immediately" and `disconnect()` with no arguments disconnects "all outgoing connections" (MDN
/// `AudioScheduledSourceNode.stop()`, `AudioNode.disconnect()`). The ids come from
/// `Playback::barge_in`, so what is stopped is exactly what the core has not already finished.
///
/// An empty slice is the common case — a keystroke between sentences — and is a no-op loop.
pub fn silence(ids: &[u64]) {
    NODES.with_borrow_mut(|nodes| {
        for id in ids {
            if let Some(node) = nodes.remove(id) {
                // A node that was never started throws `InvalidStateError` on `stop()`; swallowing it
                // is what the prior art's `try {} catch {}` does at `:33`, and there is nothing to
                // report — the node is gone either way.
                //
                // **`AudioScheduledSourceNode::stop`, not `AudioBufferSourceNode::stop`.** web-sys
                // 0.3.106 marks the subclass's own copy of the inherited `stop`/`stop_with_when`
                // `#[deprecated]` and this crate's lint gate has no room for a new warning; the parent's
                // is the same JS method, reached through the `Deref` that `extends` generates. Task 3.3
                // found the same trap in the plan's signature tables and recorded it.
                let _ = web_sys::AudioScheduledSourceNode::stop(&node);
                let _ = node.disconnect();
            }
        }
    });
}

/// A node whose audio has ended (`Playback::finished` reported its id): it is never started again, so
/// dropping the reference is all there is to do.
///
/// The prior art hangs this on the node's own `onended` (`pet/src/ui/pet.ts:44`). From Rust that is a
/// `Closure` per node and a lifetime problem `wasm-bindgen` documents rather than solves (`forget()`
/// leaks it for the life of the page; storing it in the entry the callback removes drops it while it is
/// running), so the end time the core already knows is what drives this instead. `ended` is also not
/// evidence that a sentence was heard — MDN documents that `stop()` fires it too — and the v1 spec
/// §5.3's history rule (`spec:166-171`) is about exactly that distinction.
pub fn forget(id: u64) {
    NODES.with_borrow_mut(|nodes| {
        nodes.remove(&id);
    });
}
