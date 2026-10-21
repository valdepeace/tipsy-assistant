// web/client/mic-worklet.js
//
// One processor, named "mic-frames". It does exactly two things: it forwards each render quantum to
// the page's Rust half, and it keeps itself alive. The 320-sample boundary is NOT here: 320 is not a
// multiple of the 128-frame quantum, so the accumulator lives in Rust where it has tests
// (crates/tipsy-web/src/mic.rs).
//
// A real file and not a Blob URL: the research note could not verify that `addModule` accepts a Blob
// URL (ratzilla-and-wasm-audio.md §4.4, "UNVERIFIED (single fetcher)"), and this plan does not bet on
// an unverified API.
//
// **The path this file is served at is `WORKLET_URL` in crates/tipsy-web/src/mic.rs**, and the two
// are checked against each other by that module's own test: the embed serves the repository's `web/`
// under `/assets/`, so a file at `web/client/mic-worklet.js` is `/assets/client/mic-worklet.js` —
// not the `/assets/mic-worklet.js` the plan's constant spells, which would name a file at
// `web/mic-worklet.js` that nothing creates.
class MicFrames extends AudioWorkletProcessor {
  process(inputs) {
    const channels = inputs[0];
    // The size is *checked*, never assumed: MDN's own warning on `process()` is that the render
    // quantum may not be 128 frames (ratzilla-and-wasm-audio.md §4.5), so an empty input simply
    // forwards nothing and the node keeps running.
    if (channels && channels.length > 0 && channels[0] && channels[0].length > 0) {
      // slice(0) copies: the render thread reuses these arrays on the next call. The block is posted
      // as the array of channels it is; the Rust side downmixes (audio/device.rs:186-189).
      this.port.postMessage(channels.map((channel) => channel.slice(0)));
    }
    return true; // keep the node alive
  }
}

registerProcessor("mic-frames", MicFrames);
