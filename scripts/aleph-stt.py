#!/usr/bin/env python3
"""Aleph STT — Speech-to-text using moonshine-tiny with streaming."""
import sys, os, json, argparse, itertools, torch
import numpy as np
from transformers import AutoModelForSpeechSeq2Seq, AutoProcessor

MODEL_PATH = os.path.expanduser("~/.local/share/aleph/models/moonshine-tiny")
SAMPLE_RATE = 16000

def load_model():
    if not os.path.exists(MODEL_PATH):
        # Download on first use
        model = AutoModelForSpeechSeq2Seq.from_pretrained("UsefulSensors/moonshine-tiny")
        processor = AutoProcessor.from_pretrained("UsefulSensors/moonshine-tiny")
        model.save_pretrained(MODEL_PATH)
        processor.save_pretrained(MODEL_PATH)
    else:
        model = AutoModelForSpeechSeq2Seq.from_pretrained(MODEL_PATH)
        processor = AutoProcessor.from_pretrained(MODEL_PATH)
    model.eval()
    if torch.cuda.is_available():
        model = model.to("cuda")
    return model, processor

def transcribe_file(model, processor, audio_path):
    import soundfile as sf
    audio, sr = sf.read(audio_path)
    if sr != SAMPLE_RATE:
        import librosa
        audio = librosa.resample(audio, orig_sr=sr, target_sr=SAMPLE_RATE)
    inputs = processor(audio, sampling_rate=SAMPLE_RATE, return_tensors="pt")
    if torch.cuda.is_available():
        inputs = {k: v.to("cuda") for k, v in inputs.items()}
    with torch.no_grad():
        generated = model.generate(**inputs, max_length=448)
    text = processor.decode(generated[0], skip_special_tokens=True)
    return text.strip()

def transcribe_streaming(model, processor):
    """Streaming transcription from microphone."""
    import pyaudio
    import webrtcvad
    from collections import deque

    vad = webrtcvad.Vad(2)  # aggressiveness 2
    audio_buffer = deque()
    in_speech = False
    silence_frames = 0
    SAMPLE_WIDTH = 2  # 16-bit
    CHUNK = 480  # 30ms at 16kHz
    FORMAT = pyaudio.paInt16
    CHANNELS = 1
    RATE = SAMPLE_RATE

    p = pyaudio.PyAudio()
    stream = p.open(format=FORMAT, channels=CHANNELS, rate=RATE,
                    input=True, frames_per_buffer=CHUNK,
                    stream_callback=None)

    print("  Listening... (speak now, press Ctrl+C to stop)", file=sys.stderr)

    try:
        while True:
            frame = stream.read(CHUNK, exception_on_overflow=False)
            is_speech = vad.is_speech(frame, RATE)

            if is_speech:
                audio_buffer.append(frame)
                in_speech = True
                silence_frames = 0
            elif in_speech:
                silence_frames += 1
                audio_buffer.append(frame)
                if silence_frames > 20:  # ~600ms of silence = end of utterance
                    # Process the utterance
                    raw = b''.join(audio_buffer)
                    audio = np.frombuffer(raw, dtype=np.int16).astype(np.float32) / 32768.0
                    inputs = processor(audio, sampling_rate=SAMPLE_RATE, return_tensors="pt")
                    if torch.cuda.is_available():
                        inputs = {k: v.to("cuda") for k, v in inputs.items()}
                    with torch.no_grad():
                        generated = model.generate(**inputs, max_length=448)
                    text = processor.decode(generated[0], skip_special_tokens=True).strip()
                    result = json.dumps({"text": text, "partial": False})
                    sys.stdout.write(result + "\n")
                    sys.stdout.flush()
                    audio_buffer.clear()
                    in_speech = False
                    silence_frames = 0
    except KeyboardInterrupt:
        pass
    finally:
        stream.stop_stream()
        stream.close()
        p.terminate()

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--file", help="Audio file to transcribe")
    parser.add_argument("--stream", action="store_true", help="Streaming mode from mic")
    args = parser.parse_args()

    model, processor = load_model()

    if args.stream:
        transcribe_streaming(model, processor)
    elif args.file:
        text = transcribe_file(model, processor, args.file)
        print(json.dumps({"text": text, "partial": False}))
    else:
        # Read raw PCM from stdin
        raw = sys.stdin.buffer.read()
        if raw:
            audio = np.frombuffer(raw, dtype=np.int16).astype(np.float32) / 32768.0
            inputs = processor(audio, sampling_rate=SAMPLE_RATE, return_tensors="pt")
            if torch.cuda.is_available():
                inputs = {k: v.to("cuda") for k, v in inputs.items()}
            with torch.no_grad():
                generated = model.generate(**inputs, max_length=448)
            text = processor.decode(generated[0], skip_special_tokens=True).strip()
            print(json.dumps({"text": text, "partial": False}))
