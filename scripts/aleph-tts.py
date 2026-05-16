#!/usr/bin/env python3
"""Aleph TTS — Text-to-speech using Kitten TTS Mini with streaming."""
import sys, os, json, argparse, struct, time, threading
import numpy as np
import soundfile as sf

SAMPLE_RATE = 24000

_model = None
_lock = threading.Lock()

def get_model():
    global _model
    if _model is None:
        with _lock:
            if _model is None:
                from kittentts import KittenTTS
                model_name = os.environ.get("ALEPH_TTS_MODEL", "KittenML/kitten-tts-mini-0.8")
                voice = os.environ.get("ALEPH_TTS_VOICE", "Jasper")
                m = KittenTTS(model_name)
                _model = (m, voice)
    return _model

def synthesize(text, out_path=None):
    """Synthesize text to audio."""
    m, voice = get_model()
    audio = m.generate(text, voice=voice)
    if out_path:
        sf.write(out_path, audio, SAMPLE_RATE)
        dur = len(audio) / SAMPLE_RATE
        print(json.dumps({"path": out_path, "duration": dur}), flush=True)
    else:
        # Raw PCM to stdout
        pcm = (audio * 32767).astype(np.int16).tobytes()
        sys.stdout.buffer.write(pcm)
        sys.stdout.buffer.flush()

def synthesize_streaming(text):
    """Split text into sentences and synthesize each, writing length-prefixed PCM."""
    m, voice = get_model()
    sentences = [s.strip() for s in text.replace("!", ".").replace("?", ".").split(".")
                 if s.strip()]
    for sentence in sentences:
        audio = m.generate(sentence, voice=voice)
        if len(audio) == 0:
            continue
        pcm = (audio * 32767).astype(np.int16).tobytes()
        sys.stdout.buffer.write(struct.pack('<I', len(audio)))
        sys.stdout.buffer.write(pcm)
        sys.stdout.buffer.flush()

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--text", help="Text to speak")
    parser.add_argument("--file", help="Output WAV file")
    parser.add_argument("--stream", action="store_true", help="Streaming mode")
    args = parser.parse_args()

    if args.stream and args.text:
        synthesize_streaming(args.text)
    elif args.file and args.text:
        synthesize(args.text, args.file)
    elif args.text:
        synthesize(args.text)
    else:
        text = sys.stdin.read().strip()
        if text:
            synthesize(text)
