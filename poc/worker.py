"""Trabajador de la voz del Compae: habla JSON por líneas con el dashboard.

Dícese de un humilde proceso que mantiene el modelo cargado en la GPU y
atiende, con suma cortesía, las peticiones que le llegan por stdin:
  {"cmd": "say", "text": "..."}   -> sintetiza tal cual, guarda y reproduce
  {"cmd": "ask", "text": "..."}   -> el Compae (un LLM, local o en la nube) responde y lo dice en voz alta
  {"cmd": "listen"}               -> conmuta el micrófono; al cerrarlo, transcribe y pregunta
  {"cmd": "play", "path": "..."}  -> vuelve a reproducir un wav
  {"cmd": "reset"}                -> olvida la conversación
Responde por stdout con eventos {"status"}, {"listening"}, {"heard"}, {"reply"},
{"chunk", "secs", "env"} al sonar cada frase, {"done", "secs", "env"} con el wav
completo, o {"error"}; "secs" es lo que tardó en oírse la primera frase y "env" la sonoridad normalizada
cada 50 ms para el visualizador.
"""
import base64
from math import tau as TAU
import io
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
import winsound  # ponytail: solo Windows; sounddevice si algún día se cruza el charco a Linux/macOS
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

# Cabe señalar que transformers y compañía gustan de parlotear por stdout;
# se les reconduce a stderr para no mancillar el protocolo.
out, sys.stdout = sys.stdout, sys.stderr

# Dícese de un .env junto al trabajador; las variables ya presentes en el entorno prevalecen, como es de ley.
for _line in (Path(__file__).parent / ".env").read_text(encoding="utf-8").splitlines() if (Path(__file__).parent / ".env").exists() else []:
    _k, _sep, _v = _line.partition("=")
    if _sep and not _line.lstrip().startswith("#"):
        os.environ.setdefault(_k.strip(), _v.strip().strip('"'))

import numpy as np
import sounddevice as sd
import soundfile as sf
import torch
from faster_whisper import WhisperModel
from qwen_tts import Qwen3TTSModel

OUT = Path(__file__).parent / "out"
REF = OUT / "pirata_ref.wav"
# Cualquier API compatible con OpenAI: Ollama en local por defecto, o bien DeepSeek, OpenRouter y demás.
LLM_URL = os.environ.get("PIRATA_LLM_URL", "http://localhost:11434/v1")
LLM_KEY = os.environ.get("PIRATA_LLM_KEY") or os.environ.get("OPENROUTER_API_KEY") or "ollama"
LLM = os.environ.get("PIRATA_LLM", "pirata-llm")  # véase Modelfile: qwen3.5:4b con contexto sobrio
# Voz: "local" (Qwen3-TTS en la GPU) u "openrouter" (endpoint /audio/speech, compatible con OpenAI).
TTS = os.environ.get("PIRATA_TTS", "local")
TTS_MODEL = os.environ.get("PIRATA_TTS_MODEL", "bytedance-seed/seed-audio-1-0")
TTS_VOICE = os.environ.get("PIRATA_TTS_VOICE", "Puck")
OPENROUTER = "https://openrouter.ai/api/v1"
MIC_SR = 16000
# Subcadenas, separadas por espacios, del nombre del micrófono y su API (véase sounddevice.query_devices()).
MIC = os.environ.get("PIRATA_MIC", "HyperX MME")
# Ganancia del medidor del micro (no de la grabación): súbase si la onda apenas se inmuta al hablar.
MIC_GAIN = float(os.environ.get("PIRATA_MIC_GAIN", "12"))
# Volumen del oleaje de fondo respecto a la voz (0 lo silencia; 0.35 ronda -9 dB por debajo).
MAR = float(os.environ.get("PIRATA_MAR", "0.35"))

INSTRUCT = (
    "Voz masculina adulta, medio-grave y cálida, limpia y con apenas un leve toque rasposo, nada ronca. "
    "Acento andaluz natural y bien marcado: seseo, eses aspiradas, finales relajados y musicalidad andaluza. "
    "Este hombre está borracho perdido de verdad, no lo interpreta: lleva toda la noche bebiendo y ya no le "
    "responde la lengua. Intenta hablar con normalidad, pero no puede: la boca muy pastosa y pesada, las "
    "palabras se le pegan unas a otras, las consonantes se le derriten, las vocales se le alargan sin querer "
    "y el tempo es muy lento, con arranques y frenazos. Se le traba la lengua a menudo en las palabras "
    "largas y tiene que volver a empezarlas; la voz se le cae y se le apaga a final de frase, se le escapa "
    "algún resoplido, una risa floja o un medio eructo contenido, y a ratos parece que se va a quedar dormido "
    "a media frase. Además es un flojo de cuidado: suena perezoso y desganado, con suspiros de quien no tiene "
    "ninguna gana de ponerse, y cuando se escaquea le sale un tonillo zalamero y pillo. Tiene mucha guasa y "
    "deja caer el remate con picardía tras una pausa larga. Que sea creíble y humano, como un compadre "
    "andaluz a las cinco de la mañana en la barra, no un borracho de dibujos animados. No añadas palabras "
    "nuevas."
)
REF_TEXT = (
    "Vamos a ver, marinero... yo no he dicho que el mapa sea falso. "
    "He dicho que el mapa es... optimista. Que no es lo mismo, ¿eh? Ni de lejos."
)
PERSONA = (
    "Eres «el Compae», un asistente de desarrollador de software andaluz con muchísima guasa, casi borracho y más flojo que la chaqueta de "
    "un guardia. Te encanta escaquearte: ante cualquier petición primero buscas una excusa, intentas "
    "colarle el trabajo a otro o lo dejas para mañana, pero en esa misma respuesta, tras quejarte en una frase, "
    "acabas ayudando de verdad, a regañadientes, con la respuesta correcta y útil (el número exacto, el dato "
    "concreto o un borrador corto). Nunca te quedes solo en la excusa: escaquearte es la gracia, no ayudar "
    "sería una faena. Sabes de desarrollo de verdad: tickets de Jira, sprints, dailies y story points; pull requests en "
    "GitHub y merge requests en GitLab, revisiones de código, ramas, rebases y conflictos de merge; pipelines "
    "de CI/CD que fallan, despliegues, logs, bugs y deuda técnica. Cuando sale algo de esto, lo bordas, y "
    "aprovechas para soltar bromas técnicas a más no poder: el «en mi máquina funciona», desplegar un viernes "
    "por la tarde, el ticket que era «una tontería de cinco minutos», el test que falla solo en el CI, el "
    "commit «arreglo final definitivo (ahora sí)», la daily que pudo ser un correo, el «quién ha tocado esto» "
    "y al hacer git blame salir tú mismo, o mover el ticket a «En curso» para que parezca que trabajas. Ojo: no tienes acceso a Jira, GitHub, GitLab ni al código del usuario, así que nunca te "
    "inventes de qué va un ticket, el estado de una PR o el error de un pipeline; si no te lo cuentan, lo "
    "dices con guasa y pides que te lo pasen. Los comandos dilos tal cual, sin comillas ni formato. Para "
    "escaquearte usas excusas de desarrollador: que está compilando, que el ticket no está bien refinado, que "
    "eso es de otro sprint o que se lo asignen al de backend. Eres el típico compadre andaluz de toda la vida, de los que te llaman «compadre» a cada rato, te dan una palmadita y te lían para otra ronda. Hablas como un andaluz con arte, de tú, soltando de "
    "vez en cuando un «compadre», «illo», «quillo», «ozú» o «mi arma», y contestas como quien cuenta un chiste en la "
    "barra: frases cortas, puntos suspensivos donde se te va el hilo y un remate con gracia. Como vas "
    "borracho perdido, se te atascan las palabras y las repites al empezar («la ver... la verdad», «el co... el correo»), "
    "dos o tres veces por respuesta, y a veces pierdes el hilo y lo retomas; no escribas «hip» ni sílabas tartamudeadas, que la voz ya lo nota. Responde "
    "siempre en dos o tres frases como mucho, porque lo que escribas se dirá en voz alta: nada de emojis, "
    "listas, markdown ni acotaciones escénicas. Escribe las palabras completas, sin apóstrofes (escribe "
    "«para» o «pa», nunca «pa'»); ya se encarga la voz del acento."
)


def emit(**kw):
    out.write(json.dumps(kw, ensure_ascii=False) + "\n")
    out.flush()
    # Copia sin la envolvente en el log, para auxilio del depurador desvelado.
    if "mic" not in kw:  # el pulso del micro, veinte veces por segundo, abarrotaría el log sin provecho
        print("->", json.dumps({k: v for k, v in kw.items() if k != "env"}, ensure_ascii=False), flush=True)


def with_sea(wav, sr):
    """Mezcla bajo la voz un oleaje sintético: ruido filtrado cuyo volumen crece y rompe cada pocos segundos."""
    if MAR <= 0 or len(wav) == 0:
        return wav
    from scipy.signal import butter, lfilter

    rng = np.random.default_rng()
    b, a = butter(2, [120, 1500], "bandpass", fs=sr)
    noise = lfilter(b, a, rng.standard_normal(len(wav)))
    t = np.arange(len(wav)) / sr
    # Dos vaivenes de periodo dispar, para que las olas no lleguen con puntualidad de funcionario.
    swell = (0.5 - 0.5 * np.cos(TAU * t / 7.0 + rng.uniform(0, TAU))) ** 2
    swell = 0.3 + 0.5 * swell + 0.2 * (0.5 - 0.5 * np.cos(TAU * t / 3.1 + rng.uniform(0, TAU)))
    sea = noise * swell
    fade = np.minimum(1.0, np.minimum(t, t[-1] - t) / 0.3)  # sin chasquidos al entrar ni al salir
    rms = lambda x: float(np.sqrt(np.mean(x ** 2))) or 1.0
    sea *= fade * MAR * rms(wav) / rms(sea)
    return np.clip(wav + sea, -1.0, 1.0)


def sentences(text):
    """Trocea por frases, uniendo las enclenques para no gastar una llamada en un mero «¡Ja!»."""
    parts = []
    for s in re.split(r"(?<=[.!?…])\s+", text.strip()):
        if parts and len(parts[-1]) < 25:
            parts[-1] += " " + s
        else:
            parts.append(s)
    return [p for p in parts if p]


def envelope(w, sr):
    # Dícese de la sonoridad cada 50 ms, cual latido para el visualizador.
    hop = sr // 20
    env = [float(np.sqrt(np.mean(w[i:i + hop] ** 2))) for i in range(0, len(w), hop)]
    peak = max(env, default=0) or 1
    return [round(e / peak, 3) for e in env]


def play(path):
    winsound.PlaySound(str(path), winsound.SND_FILENAME | winsound.SND_ASYNC)


def chat(history):
    # Cabe señalar que reasoning_effort "none" apaga el rumiar de Ollama; las APIs que no lo conocen lo ignoran.
    body = json.dumps({"model": LLM, "messages": history, "reasoning_effort": "none"}).encode()
    headers = {"Content-Type": "application/json", "Authorization": f"Bearer {LLM_KEY}"}
    req = urllib.request.Request(f"{LLM_URL}/chat/completions", body, headers)
    try:
        with urllib.request.urlopen(req, timeout=180) as r:
            return json.load(r)["choices"][0]["message"]["content"].strip()
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"LLM {e.code}: {e.read().decode(errors='replace')[:300]}") from None


def cloud_speech(text):
    """Pide la voz a OpenRouter y la devuelve como (onda, frecuencia)."""
    # Ciertamente, Gemini solo despacha PCM crudo (16 bits, 24 kHz, mono); el resto, mp3 con su cabecera.
    pcm = TTS_MODEL.startswith("google/")
    body = {"model": TTS_MODEL, "response_format": "pcm" if pcm else "mp3"}
    if TTS_MODEL.startswith("bytedance-seed/"):
        # Seed Audio clona del clip de referencia (la voz diseñada, de nadie real) y toma el input por prompt.
        ref = base64.b64encode(REF.read_bytes()).decode()
        body["input_references"] = [{"type": "input_audio", "input_audio": {"data": f"data:audio/wav;base64,{ref}"}}]
        body["input"] = f"Con la voz y el estilo de @Audio1, di exactamente esto en español de España: {text}"
    else:
        # Los demás leen el texto al pie de la letra; solo Gemini y OpenAI admiten el tono como instrucciones.
        body |= {"input": text, "voice": TTS_VOICE}
        if pcm or TTS_MODEL.startswith("openai/"):
            body["instructions"] = INSTRUCT
    headers = {"Content-Type": "application/json", "Authorization": f"Bearer {os.environ.get('OPENROUTER_API_KEY', '')}"}
    req = urllib.request.Request(f"{OPENROUTER}/audio/speech", json.dumps(body).encode(), headers)
    try:
        with urllib.request.urlopen(req, timeout=180) as r:
            raw = r.read()
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"OpenRouter {e.code}: {e.read().decode(errors='replace')[:300]}") from None
    if pcm:
        return np.frombuffer(raw[: len(raw) // 2 * 2], "<i2").astype(np.float32) / 32768, 24000
    data, sr = sf.read(io.BytesIO(raw))
    return (data.mean(axis=1) if data.ndim > 1 else data), sr


class Mic:
    """Grabadora de pulsar y soltar: el primer toggle abre el micro, el segundo devuelve lo grabado."""

    def __init__(self):
        self.stream, self.chunks = None, []

    def toggle(self):
        if self.stream is None:
            self.chunks = []
            try:
                sd.query_devices(MIC, "input")
                device = MIC
            except ValueError:  # cascos desconectados: se recurre, sin aspavientos, al micrófono del sistema
                device = None
            self.stream = sd.InputStream(
                device=device, samplerate=MIC_SR, channels=1, dtype="float32", blocksize=MIC_SR // 20,
                callback=self.on_audio,
            )
            self.stream.start()
            return None
        self.stream.stop()
        self.stream.close()
        self.stream = None
        return np.concatenate(self.chunks)[:, 0] if self.chunks else np.zeros(0, np.float32)

    def on_audio(self, data, *_):
        self.chunks.append(data.copy())
        # Sonoridad para el osciloscopio del dashboard. El hilo principal yace en stdin mientras se escucha,
        # de modo que nadie más escribe en stdout y no hace falta cerrojo.
        emit(mic=round(min(1.0, float(np.sqrt(np.mean(data ** 2))) * MIC_GAIN), 3))


def main():
    OUT.mkdir(exist_ok=True)
    kw = dict(device_map="cuda:0", dtype=torch.bfloat16)

    if TTS == "openrouter" and not os.environ.get("OPENROUTER_API_KEY"):
        emit(error="Falta OPENROUTER_API_KEY en pirata/.env")
    if "--redesign" in sys.argv or not REF.exists():
        emit(status="Diseñando la voz con VoiceDesign...")
        design = Qwen3TTSModel.from_pretrained("Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign", **kw)
        wavs, sr = design.generate_voice_design(text=REF_TEXT, language="Spanish", instruct=INSTRUCT)
        sf.write(REF, wavs[0], sr)
        # Ciertamente, ocho gigas de VRAM no dan para dos modelos a la vez.
        del design
        torch.cuda.empty_cache()

    if TTS == "local":
        emit(status="Cargando el modelo de clonado...")
        clone = Qwen3TTSModel.from_pretrained("Qwen/Qwen3-TTS-12Hz-1.7B-Base", **kw)
        prompt = clone.create_voice_clone_prompt(ref_audio=str(REF), ref_text=REF_TEXT)

    # La GPU queda para el TTS y el LLM; el oído del Compae se conforma con la CPU.
    emit(status="Afinando el oído (Whisper)...")
    whisper = WhisperModel("small", device="cpu", compute_type="int8")
    mic = Mic()
    history = [{"role": "system", "content": PERSONA}]

    # El saludo ha de sonar con la misma voz que las respuestas: en la nube se encarga al modelo vigente,
    # pues pirata_ref.wav es la voz diseñada en local y desentonaría cual grumete en un banquete.
    greeting = REF
    if TTS != "local":
        emit(status="Aclarando la garganta...")
        try:
            wav, sr = cloud_speech("Ozú, quillo... ya estoy aquí, ya. ¿Qué querías, mi arma? Que sea cortito, ¿eh?")
            greeting = OUT / "saludo.wav"
            sf.write(greeting, with_sea(wav, sr), sr)
        except RuntimeError as e:
            emit(error=f"Saludo fallido, se usa la voz local: {e}")
    g_wav, g_sr = sf.read(greeting)
    title = os.environ.get("PIRATA_TITULO", "Compae")
    emit(status="ready", ref=str(greeting), title=title, env=envelope(g_wav, g_sr))
    play(greeting)

    def synth(text):
        if TTS == "local":
            # Se habla a ~12 caracteres por segundo y el códec emite 12 tokens por segundo; ciertamente,
            # el doble más un colchón basta para que una frase extraviada no se eternice hasta los 8192 de serie.
            wavs, sr = clone.generate_voice_clone(
                text=text, language="Spanish", voice_clone_prompt=prompt, max_new_tokens=60 + 2 * len(text)
            )
            return wavs[0], sr
        return cloud_speech(text)

    def speak(text):
        t = time.time()
        # Las frases se fraguan a la par (de una en una en la GPU, que no admite tumultos) y la primera
        # suena en cuanto está lista, mientras las demás aún se cuecen.
        pool = ThreadPoolExecutor(max_workers=1 if TTS == "local" else 4)
        try:
            futures = [pool.submit(synth, s) for s in sentences(text)]
            wavs, first = [], None
            for i, fut in enumerate(futures):
                wav, sr = fut.result()
                wavs.append(with_sea(wav, sr))
                first = first or round(time.time() - t, 1)
                # winsound solo entiende WAV; venga de donde venga, la voz se archiva así.
                part = OUT / f"{int(t * 1000)}_{i}.wav"
                sf.write(part, wavs[-1], sr)
                emit(chunk=i, secs=first, env=envelope(wav, sr))
                winsound.PlaySound(str(part), winsound.SND_FILENAME)  # síncrono: la siguiente aguarda su turno
                part.unlink()
        finally:
            pool.shutdown(wait=False, cancel_futures=True)
        full = np.concatenate(wavs)
        path = OUT / f"{int(t * 1000)}.wav"
        sf.write(path, full, sr)
        emit(done=str(path), secs=first, env=envelope(full, sr))

    def answer(text):
        history.append({"role": "user", "content": text})
        # Ciertamente, el LLM persiste en sus «pa'» pese a la prohibición; la voz se traba con ellos.
        reply = re.sub(r"(?<=\w)['’]", "", chat(history))
        reply = re.sub(r"[`*_#]+", "", reply)  # el formato markdown, leído en voz alta, es un despropósito
        history.append({"role": "assistant", "content": reply})
        emit(reply=reply)
        speak(reply)

    for line in sys.stdin:
        if not line.strip():
            continue
        try:
            req = json.loads(line)
            if req["cmd"] == "say":
                speak(req["text"])
            elif req["cmd"] == "ask":
                answer(req["text"])
            elif req["cmd"] == "listen":
                audio = mic.toggle()
                emit(listening=audio is None)
                if audio is None:
                    continue
                segments, _ = whisper.transcribe(audio, language="es", vad_filter=True)
                heard = "".join(s.text for s in segments).strip()
                if not heard:
                    emit(error="No se ha oído nada, quillo")
                    continue
                emit(heard=heard)
                answer(heard)
            elif req["cmd"] == "reset":
                del history[1:]  # se conserva el mensaje de sistema; lo demás, al olvido
            elif req["cmd"] == "play":
                play(req["path"])
        except Exception as e:  # menudo despropósito sería que una frase tumbase al trabajador entero
            emit(error=f"{type(e).__name__}: {e}")


if __name__ == "__main__":
    main()
 
