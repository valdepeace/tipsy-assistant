//! Portable PCM helpers. Everything here is mono `f32` in −1.0..=1.0 unless stated otherwise.

use std::io::Cursor;

#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl Pcm {
    pub fn duration_ms(&self) -> u64 {
        if self.sample_rate == 0 {
            return 0;
        }
        self.samples.len() as u64 * 1000 / self.sample_rate as u64
    }
}

pub fn f32_to_i16(samples: &[f32]) -> Vec<i16> {
    samples.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16).collect()
}

pub fn i16_to_f32(samples: &[i16]) -> Vec<f32> {
    samples.iter().map(|s| *s as f32 / 32768.0).collect()
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Linear-interpolation resampler; good enough for speech going to a recogniser.
pub fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if samples.is_empty() || from == 0 || to == 0 {
        return Vec::new();
    }
    if from == to {
        return samples.to_vec();
    }
    let out_len = (samples.len() as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    (0..out_len)
        .map(|i| {
            let pos = i as f64 * step;
            let idx = pos.floor() as usize;
            let frac = (pos - idx as f64) as f32;
            let a = samples[idx.min(samples.len() - 1)];
            let b = samples[(idx + 1).min(samples.len() - 1)];
            a + (b - a) * frac
        })
        .collect()
}

/// Loudness every 50 ms, normalised so the loudest window is 1.0.
pub fn envelope(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    let hop = (sample_rate / 20).max(1) as usize;
    let env: Vec<f32> = samples.chunks(hop).map(rms).collect();
    let peak = env.iter().cloned().fold(0.0, f32::max);
    if peak <= 0.0 {
        return env;
    }
    env.into_iter().map(|e| e / peak).collect()
}

pub fn encode_wav_16k(pcm: &Pcm) -> Vec<u8> {
    let samples = resample_linear(&pcm.samples, pcm.sample_rate, 16000);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec).expect("in-memory wav writer");
        for s in f32_to_i16(&samples) {
            writer.write_sample(s).expect("in-memory wav write");
        }
        writer.finalize().expect("in-memory wav finalize");
    }
    cursor.into_inner()
}

pub fn pcm_s16le_to_pcm(bytes: &[u8], sample_rate: u32) -> Pcm {
    let ints: Vec<i16> = bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    Pcm { samples: i16_to_f32(&ints), sample_rate }
}

/// Decodes MP3 or WAV bytes to mono PCM (channels are averaged).
pub fn decode_audio(bytes: Vec<u8>) -> Result<Pcm, String> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let probed = symphonia::default::get_probe()
        .format(&Hint::new(), mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("unrecognised audio: {e}"))?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("audio has no track")?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("unsupported codec: {e}"))?;
    let mut samples = Vec::new();
    let mut sample_rate = 0;
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet).map_err(|e| format!("decode error: {e}"))?;
        let spec = *decoded.spec();
        sample_rate = spec.rate;
        let channels = spec.channels.count().max(1);
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buf.copy_interleaved_ref(decoded);
        for frame in buf.samples().chunks(channels) {
            samples.push(frame.iter().sum::<f32>() / channels as f32);
        }
    }
    if sample_rate == 0 {
        return Err("audio contained no samples".into());
    }
    Ok(Pcm { samples, sample_rate })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, ms: u32) -> Vec<f32> {
        let n = (rate as u64 * ms as u64 / 1000) as usize;
        (0..n).map(|i| (i as f32 * freq * std::f32::consts::TAU / rate as f32).sin() * 0.5).collect()
    }

    #[test]
    fn i16_conversion_clamps_and_roundtrips() {
        assert_eq!(f32_to_i16(&[0.0, 1.0, -1.0, 2.0, -2.0]), vec![0, 32767, -32767, 32767, -32767]);
        let back = i16_to_f32(&[0, 16384, -16384]);
        assert!((back[1] - 0.5).abs() < 0.001 && (back[2] + 0.5).abs() < 0.001);
    }

    #[test]
    fn resample_changes_length_proportionally() {
        let s = sine(440.0, 48000, 1000);
        assert_eq!(resample_linear(&s, 48000, 16000).len(), 16000);
        assert_eq!(resample_linear(&s, 48000, 48000).len(), 48000);
        assert!(resample_linear(&[], 48000, 16000).is_empty());
    }

    #[test]
    fn envelope_has_one_value_per_50ms_and_peak_one() {
        let mut s = vec![0.0f32; 24000 / 20]; // 50 ms of silence
        s.extend(sine(220.0, 24000, 100)); // 100 ms of tone
        let env = envelope(&s, 24000);
        assert_eq!(env.len(), 3);
        assert_eq!(env[0], 0.0);
        assert!((env.iter().cloned().fold(0.0, f32::max) - 1.0).abs() < 1e-6);
        assert!(envelope(&vec![0.0; 2400], 24000).iter().all(|v| *v == 0.0));
    }

    #[test]
    fn duration_in_ms() {
        assert_eq!(Pcm { samples: vec![0.0; 24000], sample_rate: 24000 }.duration_ms(), 1000);
        assert_eq!(Pcm { samples: vec![], sample_rate: 0 }.duration_ms(), 0);
    }

    #[test]
    fn wav_roundtrip_through_decoder() {
        let pcm = Pcm { samples: sine(440.0, 48000, 500), sample_rate: 48000 };
        let wav = encode_wav_16k(&pcm);
        assert_eq!(&wav[0..4], b"RIFF");
        let back = decode_audio(wav).unwrap();
        assert_eq!(back.sample_rate, 16000);
        assert_eq!(back.samples.len(), 8000);
    }

    #[test]
    fn raw_s16le_decoding() {
        let bytes = [0x00, 0x40, 0x00, 0xC0]; // 16384, -16384
        let pcm = pcm_s16le_to_pcm(&bytes, 24000);
        assert_eq!(pcm.sample_rate, 24000);
        assert!((pcm.samples[0] - 0.5).abs() < 0.001 && (pcm.samples[1] + 0.5).abs() < 0.001);
        assert_eq!(pcm_s16le_to_pcm(&[1, 2, 3], 24000).samples.len(), 1); // odd trailing byte ignored
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode_audio(b"not audio at all".to_vec()).is_err());
    }
}
