#!/usr/bin/env python3
"""Aleph TTS — ONNX inference for kitten-tts-mini with streaming."""
import sys, os, json, argparse, struct
import numpy as np
import onnxruntime as ort

MODEL_DIR = os.path.expanduser("~/.local/share/aleph/models/kitten-tts-mini")
MODEL_PATH = os.path.join(MODEL_DIR, "kitten_tts_mini_v0_8.onnx")
VOICES_PATH = os.path.join(MODEL_DIR, "voices.npz")
SAMPLE_RATE = 24000

def load_model():
    if not os.path.exists(MODEL_PATH):
        print("Downloading kitten-tts ONNX model...", file=sys.stderr)
        import urllib.request
        os.makedirs(MODEL_DIR, exist_ok=True)
        base = "https://huggingface.co/KittenML/kitten-tts-mini-0.8/resolve/main"
        urllib.request.urlretrieve(f"{base}/kitten_tts_mini_v0_8.onnx", MODEL_PATH)
        urllib.request.urlretrieve(f"{base}/voices.npz", VOICES_PATH)

    session = ort.InferenceSession(MODEL_PATH)
    voices = np.load(VOICES_PATH)
    # Use first available voice, average across frames
    voice_name = list(voices.keys())[0]
    style = voices[voice_name].mean(axis=0, keepdims=True)  # (1, 256)
    return session, style.astype(np.float32)

def text_to_tokens(text):
    """Tokenize text for kitten-tts (vocab size 178, ASCII-based)."""
    tokens = [1]  # BOS
    for ch in text.lower():
        code = ord(ch)
        if 32 <= code <= 127:
            tokens.append(code)
        elif code < 256:
            # Fold extended chars into ASCII range
            tokens.append(32 + (code % 96))
        elif code < 0x110000:
            # Non-ASCII: use a hash collision approach
            tokens.append(32 + (code % 96))
    tokens.append(2)  # EOS
    return np.array([tokens], dtype=np.int64)

def synthesize(session, style, text, out_path=None):
    input_ids = text_to_tokens(text)
    speed = np.array([1.0], dtype=np.float32)

    outputs = session.run(["waveform", "duration"], {
        "input_ids": input_ids,
        "style": style,
        "speed": speed,
    })
    waveform, duration = outputs
    samples = waveform[:duration[0]]

    if out_path:
        import soundfile as sf
        sf.write(out_path, samples, SAMPLE_RATE)
        print(json.dumps({"path": out_path, "duration": len(samples) / SAMPLE_RATE}), flush=True)
    else:
        # Raw PCM to stdout
        pcm = (samples * 32767).astype(np.int16).tobytes()
        sys.stdout.buffer.write(pcm)
        sys.stdout.buffer.flush()

def synthesize_streaming(session, style, text):
    """Split text into sentences and synthesize each, writing length-prefixed PCM chunks."""
    import re
    sentences = re.split(r'(?<=[.!?])\s+', text)
    for sentence in sentences:
        sentence = sentence.strip()
        if not sentence:
            continue
        input_ids = text_to_tokens(sentence)
        speed = np.array([1.0], dtype=np.float32)
        outputs = session.run(["waveform", "duration"], {
            "input_ids": input_ids,
            "style": style,
            "speed": speed,
        })
        waveform, duration = outputs
        samples = waveform[:duration[0]]
        if len(samples) == 0:
            continue
        pcm = (samples * 32767).astype(np.int16).tobytes()
        # Write length prefix + PCM
        sys.stdout.buffer.write(struct.pack('<I', len(samples)))
        sys.stdout.buffer.write(pcm)
        sys.stdout.buffer.flush()

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--text", help="Text to speak")
    parser.add_argument("--file", help="Output WAV file")
    parser.add_argument("--stream", action="store_true", help="Streaming mode (length-prefixed PCM chunks)")
    args = parser.parse_args()

    session, style = load_model()

    if args.stream and args.text:
        synthesize_streaming(session, style, args.text)
    elif args.file and args.text:
        synthesize(session, style, args.text, args.file)
    elif args.text:
        synthesize(session, style, args.text)
    else:
        text = sys.stdin.read().strip()
        if text:
            synthesize(session, style, text)
