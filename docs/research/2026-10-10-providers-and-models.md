# Providers and models

> A research notebook, not a specification. It records what was known on 2026-10-10 about cloud
> providers and locally runnable models for the assistant's three capabilities: **chat** (the
> LLM), **speech** (text-to-speech) and **transcription** (speech-to-text). Prices and catalogues
> change often: verify everything before relying on it.

## Decisions already taken for v1

- Everything in Rust, everything in the cloud. Python only for local speech, not before v2.
- An OpenAI-compatible base connector (`/chat/completions`, `/audio/speech`,
  `/audio/transcriptions`), configured per capability (URL, key and model for each one).
- Native connectors in v1: **Google Gemini** (chat and speech) and **ElevenLabs** (speech and
  transcription).
- In v1.1: **Deepgram** (transcription and speech) and **Anthropic** (chat).

## Cloud: speech

Reference: a typical reply, about 200 characters and 12 s of audio. Prices from the OpenRouter
models API, queried on 2026-10-08.

| Model (OpenRouter slug) | Price | Per reply | Notes |
|---|---|---|---|
| `fish-audio/s2.1-pro-free:free` | free | 0 | Rate limited. Quality and Spanish support unverified |
| `deepgram/flux-tts:free` | free | 0 | English voices only (`flux-*-en`) |
| `canopylabs/orpheus-3b-0.1-ft`, `sesame/csm-1b` | $7 / M chars | ~$0.0014 | English oriented |
| `google/gemini-3.8-flash-lite-tts` | $0.5 / M input tokens + $6 / M audio tokens | ~$0.002 (estimate) | Cheapest model that follows delivery instructions |
| `google/gemini-3.8-flash-tts` | $0.5 / M + $9 / M | ~$0.003 (estimate) | Used by the proof of concept. Only returns `pcm` (16-bit, 24 kHz, mono) |
| `qwen/qwen-audio-3.0-tts-flash`, `x-ai/grok-voice-tts-1.0`, `fish-audio/s1` | $15 / M chars | ~$0.003 | |
| `elevenlabs/eleven-flash-v2.5`, `eleven-turbo-v2.5` | $20 / M chars | ~$0.004 | ~0.4 s latency measured. Ignores delivery instructions |
| `deepgram/aura-2` | $30 / M chars | ~$0.006 | Spanish voices (`aura-2-alvaro-es`, `aura-2-agustina-es`…) |
| `elevenlabs/eleven-v3`, `eleven-v4` | $40 / M chars | ~$0.008 | Inline emotion tags |
| `bytedance-seed/seed-audio-1-0` | $0.0025 / s of audio | ~$0.03 | ~21 s per request regardless of length. Unusable for conversation |

The Gemini estimate assumes about 32 audio tokens per second; measure it.

Latency to first audible sentence, measured in the proof of concept: ElevenLabs Flash 0.4 s,
Gemini Flash TTS 3–6 s depending on length, Seed Audio ~21 s.

OpenRouter quirks found while building the proof of concept:

- Gemini TTS rejects `response_format: "mp3"`; it only accepts `pcm`.
- Only OpenAI and Gemini TTS models accept the `instructions` field.
- Seed Audio clones from `input_references` (up to three clips, referenced as `@Audio1`…) and
  treats `input` as a prompt, not as literal text. It cannot combine `voice` and
  `input_references`.
- Speech models list their voices in the `supported_voices` field of the models API.

## Cloud: transcription

OpenRouter (`/audio/transcriptions`) offers OpenAI (Whisper Large v3, GPT-4o Transcribe, GPT-4o
Mini Transcribe), Google (Chirp 3) and Groq (accelerated Whisper). It accepts base64 audio inside
JSON (`input_audio`) or an OpenAI-style `multipart/form-data` upload, plus an optional `language`
hint.

**Pending:** the models API does not expose per-minute prices for these models. Check each model
page on OpenRouter and the direct prices of Groq, ElevenLabs Scribe and Deepgram.

## Cloud: chat

Measured in the proof of concept with 2–3 sentence replies:

| Model | Latency | Notes |
|---|---|---|
| `google/gemini-2.5-flash` | 0.8–1.5 s | Chosen. Short replies, good Spanish |
| `openai/gpt-4o-mini` | 1.0 s | |
| `mistralai/mistral-small-3.2-24b-instruct` | 1.7 s | |
| `deepseek/deepseek-v3.2` | 2.2 s | Adds stage directions (`*draws sword*`) |
| `deepseek/deepseek-chat` | 7.5 s | Returned 429 errors due to upstream saturation |

With replies this short, chat costs fractions of a cent per reply.

## Free tiers of the native connectors

A strong argument for native connectors: anyone can try the assistant for free with their own
key. As far as known (**verify**):

- **Google AI Studio (Gemini):** free tier limited by requests per minute and per day. Check
  whether it covers the TTS models.
- **ElevenLabs:** free plan of about 10,000 characters per month.
- **Deepgram:** sign-up credit.

## Local

The OpenAI-compatible connector already covers much of the local story, as long as the user runs
a server that speaks that dialect. The application stays pure Rust; the local server is the
user's business.

### Local chat

Already supported by the OpenAI-compatible connector:

| Server | Notes |
|---|---|
| **Ollama** | `http://localhost:11434/v1`. Tested in the proof of concept with `qwen3.5:4b` and `num_ctx 4096` |
| **LM Studio** | `http://localhost:1234/v1` |
| **llama.cpp** (`llama-server`) | Exposes `/v1/chat/completions` |

Lesson from the proof of concept: with 8 GB of VRAM, a local LLM and a local TTS model do not fit
together. Either the LLM runs on the CPU (`num_gpu 0`, ~9 s per reply with a 4B model) or the TTS
runs in the cloud. Ollama's large default context made the model take 8.6 GB instead of 3.1 GB.

### Local transcription

| Model | Spanish | How to serve it | Notes |
|---|---|---|---|
| **Whisper** (`small`, `medium`, `large-v3`, `large-v3-turbo`) | Good; very good from `medium` | **speaches** (formerly faster-whisper-server) or the **whisper.cpp** server, both OpenAI-compatible | The proof of concept used `small` on the CPU with int8. Native Rust: `whisper-rs` (binds whisper.cpp; needs a C++ toolchain) |
| **NVIDIA Parakeet TDT 0.6B v3** | Yes (25 European languages) | NeMo (Python) or ONNX | Very fast. Andalusian accent quality unverified |
| **Vosk** | Yes | Native library with bindings | Light and streaming, modest accuracy |

### Local speech

Expressive voices need large models, and almost all of them live in Python.

| Model | Spanish | Cloning / design | License | Notes |
|---|---|---|---|---|
| **Qwen3-TTS 1.7B** | Yes | Voice design from a description, 3-second cloning | Apache-2.0 | Tested in the proof of concept: ~38–48 s per reply on an RTX 3060 Ti without flash-attn, far from real time. The 0.6B model should be roughly twice as fast |
| **Chatterbox** (Resemble AI) | Yes, multilingual version | Cloning, emotion exaggeration control | MIT | Strong candidate for the tipsy delivery. Verify Castilian quality |
| **Kokoro-82M** | Few Spanish voices | No | Apache-2.0 | Tiny and very fast, even on the CPU. Servable through Kokoro-FastAPI (OpenAI-compatible). Not expressive enough for the profile |
| **Piper** | Yes (`es_ES` voices) | No | MIT | Very fast on the CPU, ONNX. Sounds like an answering machine |
| **F5-TTS** | Through community fine-tunes | Cloning | Code MIT; weights CC-BY-NC | Good quality. The non-commercial weights complicate distribution |
| **XTTS-v2** (Coqui) | Yes | Cloning | CPML (non-commercial) | Reliable, but the original project is abandoned |
| **Fish-Speech / OpenAudio S1-mini** | Yes | Cloning | CC-BY-NC-SA | Very good quality. Mind the license |

For a community project the license matters: **Qwen3-TTS** (Apache-2.0) and **Chatterbox** (MIT)
are the clean candidates for v2. **Kokoro** and **Piper** work as GPU-less fallback voices.

The local model notes above come from prior knowledge, not from a fresh search; verify them before
v2.

**Cloning:** only with one's own or designed voices, never with a third party's voice without
consent. The project's reference voice (`assets/voice_ref.wav`) was designed from a text
description with Qwen3-TTS VoiceDesign and belongs to no real person.

## Pending before implementation

1. Per-minute transcription prices: Groq Whisper, OpenAI Transcribe, ElevenLabs Scribe and
   Deepgram.
2. Current free tiers of Google AI Studio (including TTS models) and ElevenLabs.
3. Whether the direct ElevenLabs API streams Flash v2.5 audio over WebSocket, and its real latency
   from Spain.
4. Castilian and Andalusian quality of Chatterbox multilingual and Parakeet v3 (for v2).
