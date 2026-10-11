//! Native audio devices: playback through rodio and microphone capture through cpal.
//!
//! Both are thin wrappers: playback queues mono PCM and never blocks the caller, capture
//! delivers mono 16 kHz frames of about 20 ms so the rest of the program only ever sees the
//! shape the protocol promises.

use std::sync::mpsc::{SyncSender, TrySendError};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample};
use rodio::buffer::SamplesBuffer;
use rodio::{Player as RodioPlayer, SampleRate};

use super::pcm::{i16_to_f32, resample_linear};

/// The rate the microphone delivers at, and the only one the transcriber is ever offered.
pub const MIC_SAMPLE_RATE: u32 = 16000;
/// Samples per microphone frame: 20 ms at [`MIC_SAMPLE_RATE`].
pub const MIC_FRAME_SAMPLES: usize = (MIC_SAMPLE_RATE as usize) / 50;
/// Microphone frames that may wait for the client before the oldest are dropped.
const MIC_QUEUE: usize = 16;

/// Mono playback on the default output device. Dropping it releases the device.
pub struct Player {
    /// Kept alive for as long as playback is wanted: dropping it closes the stream.
    sink: rodio::MixerDeviceSink,
    player: RodioPlayer,
}

impl Player {
    pub fn new() -> Result<Player, String> {
        let mut sink = rodio::stream::DeviceSinkBuilder::open_default_sink()
            .map_err(|e| format!("no audio output device: {e}"))?;
        // The client restores the terminal on exit; a "dropping device sink" note on stderr would
        // land in the middle of the restored screen.
        sink.log_on_drop(false);
        let player = RodioPlayer::connect_new(sink.mixer());
        Ok(Self { sink, player })
    }

    /// Queues one mono chunk for playback. Returns immediately; the audio thread consumes it.
    pub fn play(&self, samples: Vec<i16>, sample_rate: u32) {
        if samples.is_empty() {
            return;
        }
        let Some(rate) = SampleRate::new(sample_rate) else {
            return;
        };
        let Some(channels) = std::num::NonZeroU16::new(1) else {
            return;
        };
        let buffer = SamplesBuffer::new(channels, rate, i16_to_f32(&samples));
        self.player.append(buffer);
    }

    /// Drops everything queued but not yet played, and leaves the player able to play again.
    ///
    /// rodio 0.22.2 `Player::clear` removes every loaded source **and pauses the sink**
    /// (`player.rs:279-284`), while `append` only clears the *stopped* flag, never the paused one
    /// (`player.rs:104-115`). The client never calls rodio's `play()` anywhere else, so a cleared
    /// player would stay muted for the rest of the session: the greeting would be the only thing
    /// ever heard, and `Replay` would be silent too. Resuming the sink here is what makes the next
    /// `play()` audible; it drops nothing, because the queue was just emptied.
    pub fn stop(&self) {
        self.player.clear();
        self.player.play();
    }

    /// True while a chunk is still queued or being played.
    ///
    /// rodio counts a source until it finishes (`Done` decrements the count, `source/done.rs:54-62`),
    /// so this is exactly "clearing now would drop audio the user has not heard yet".
    pub fn playing(&self) -> bool {
        !self.player.empty()
    }

    /// How much of what was queued has actually been heard: the number of sources still queued or
    /// playing, and the position inside the one that is playing now.
    ///
    /// This is rodio's own account of the device (`Player::len`, `player.rs:325-330`, and
    /// `Player::get_pos`, `:332-342`) and not a count of what was appended, because a stream is
    /// generated faster than it is played: bytes handed over would overstate what the user heard.
    /// The two halves are kept apart — the caller knows the size of the chunks it appended, and only
    /// it can say which of them `remaining` refers to.
    ///
    /// Task 5 is the caller: `played_bytes` in the realtime module turns this into the milliseconds
    /// a `conversation.item.truncate` may claim, and every way of being unsure rounds it down.
    pub fn progress(&self) -> (usize, Duration) {
        (self.player.len(), self.player.get_pos())
    }

    /// The configuration the output device actually opened with, for the log.
    pub fn config(&self) -> (u32, u16) {
        let config = self.sink.config();
        (config.sample_rate().get(), config.channel_count().get())
    }
}

/// Live capture from the default input device.
///
/// `Mic::start` hands every 20 ms frame to `on_frame` from the audio thread. Dropping the `Mic`
/// stops capture.
pub struct Mic {
    /// Kept alive for as long as capture is wanted: dropping it closes the stream. The field is
    /// never read, which is exactly the point.
    _stream: cpal::Stream,
}

impl Mic {
    /// Starts capturing the default input device, converting to mono and resampling to 16 kHz.
    ///
    /// `on_frame` runs on the audio thread and must not block; a frame it cannot keep up with is
    /// dropped by the sender in `start_with` rather than stalling the device.
    pub fn start(mut on_frame: impl FnMut(Vec<i16>) + Send + 'static) -> Result<Mic, String> {
        // Bounded, so a client that stops draining the queue cannot grow memory without limit.
        let (tx, rx) = std::sync::mpsc::sync_channel(MIC_QUEUE);
        let mic = Self::start_with(tx)?;
        // Bridge the audio thread's sender to the caller's callback: this thread exists only to
        // hand frames on, and it ends as soon as either side goes away.
        std::thread::spawn(move || {
            while let Ok(frame) = rx.recv() {
                if !frame.is_empty() {
                    on_frame(frame);
                }
            }
        });
        Ok(mic)
    }

    /// The device half of [`Mic::start`], without the forwarding thread. Keeping it separate keeps
    /// the thread's lifetime obvious: the stream lives in the returned `Mic`.
    fn start_with(tx: SyncSender<Vec<i16>>) -> Result<Mic, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "no microphone: the system reported no default input device".to_string())?;
        let supported = device
            .default_input_config()
            .map_err(|e| format!("no microphone: the input device has no default configuration: {e}"))?;
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let channels = config.channels as usize;
        let rate = config.sample_rate;

        let error_tx = tx.clone();
        let on_error = move |e: cpal::Error| {
            // The stream is gone; make sure the forwarding thread wakes up and stops.
            let _ = error_tx.try_send(Vec::new());
            let _ = e;
        };

        let stream = match sample_format {
            SampleFormat::I16 => capture::<i16>(&device, &config, channels, rate, tx, on_error),
            SampleFormat::U16 => capture::<u16>(&device, &config, channels, rate, tx, on_error),
            SampleFormat::I32 => capture::<i32>(&device, &config, channels, rate, tx, on_error),
            SampleFormat::F32 => capture::<f32>(&device, &config, channels, rate, tx, on_error),
            SampleFormat::F64 => capture::<f64>(&device, &config, channels, rate, tx, on_error),
            other => return Err(format!("the microphone uses an unsupported sample format: {other:?}")),
        }?;
        stream.play().map_err(|e| format!("the microphone stream would not start: {e}"))?;
        Ok(Mic { _stream: stream })
    }
}

/// Builds one input stream whose callback converts `T` to mono `f32` and emits 16 kHz frames.
fn capture<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    rate: u32,
    tx: SyncSender<Vec<i16>>,
    on_error: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + cpal::FromSample<f32>,
    f32: cpal::FromSample<T>,
{
    let mut mono = Vec::new();
    let mut frame: Vec<i16> = Vec::with_capacity(MIC_FRAME_SAMPLES);
    device
        .build_input_stream(
            config.clone(),
            move |data: &[T], _| {
                mono.clear();
                mono.extend(
                    data.chunks(channels.max(1))
                        .map(|c| c.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels.max(1) as f32),
                );
                for s in resample_linear(&mono, rate, MIC_SAMPLE_RATE) {
                    frame.push((s.clamp(-1.0, 1.0) * 32767.0) as i16);
                    if frame.len() == MIC_FRAME_SAMPLES {
                        let ready = std::mem::replace(&mut frame, Vec::with_capacity(MIC_FRAME_SAMPLES));
                        // Never block the audio thread: a client that is behind loses the frame.
                        match tx.try_send(ready) {
                            Ok(()) | Err(TrySendError::Full(_)) => {}
                            Err(TrySendError::Disconnected(_)) => {}
                        }
                    }
                }
            },
            on_error,
            None,
        )
        .map_err(|e| format!("the microphone stream could not be opened: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_size_is_twenty_milliseconds() {
        assert_eq!(MIC_FRAME_SAMPLES, (MIC_SAMPLE_RATE / 50) as usize);
    }

    /// The rodio behaviour `Player::stop` is built on, pinned here because it is easy to get wrong:
    /// `clear` pauses the sink, and only `play` resumes it. No device is needed — this is a bare
    /// player with an empty queue, so `clear` returns at once instead of waiting for a source.
    #[test]
    fn a_cleared_rodio_player_stays_paused_until_it_is_resumed() {
        let (player, _queue) = RodioPlayer::new();
        assert!(!player.is_paused());
        player.clear();
        assert!(player.is_paused(), "rodio 0.22.2 `clear` pauses the sink (player.rs:279-284)");
        player.play();
        assert!(!player.is_paused(), "`play` is what lets the next `play()` be heard");
    }
}
