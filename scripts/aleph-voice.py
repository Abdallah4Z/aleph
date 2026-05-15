#!/usr/bin/env python3
"""Aleph Voice Assistant — wake word + query loop.
 
Continuously listens for "okay aleph" or "aleph" wake word,
then records your query, sends to Aleph API, and speaks the answer.

Usage:
  python3 aleph-voice.py              # wake word mode (background service)
  python3 aleph-voice.py --once       # single query (for aleph listen command)
  python3 aleph-voice.py --text "..." # process text directly
"""
import sys, os, json, time, struct, argparse, subprocess, threading
import numpy as np

SAMPLE_RATE = 16000
WAKE_WORDS = ["aleph", "okay aleph", "hey aleph", "aleph "]
API_BASE = "http://127.0.0.1:2198"
MODELS_DIR = os.path.expanduser("~/.local/share/aleph/models")
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))

# ============================================================
# Audio capture
# ============================================================

def record_seconds(duration, rate=SAMPLE_RATE):
    """Record audio for N seconds, return 16-bit PCM bytes."""
    tmp = f"/tmp/aleph_voice_{int(time.time())}.wav"
    try:
        subprocess.run(["arecord", "-q", "-f", "S16_LE", "-r", str(rate),
                       "-c", "1", "-d", str(duration), tmp],
                      timeout=duration + 5, stderr=subprocess.DEVNULL)
        with open(tmp, "rb") as f:
            wav = f.read()
        os.remove(tmp)
        return wav[44:] if len(wav) > 44 else wav
    except:
        try:
            result = subprocess.run(["parec", "--rate=16000", "--channels=1",
                                    "--format=s16le", "--record",
                                    "--duration", str(duration)],
                                   capture_output=True, timeout=duration + 5)
            return result.stdout
        except:
            return b""

def record_streaming(callback_chunk, chunk_secs=0.5):
    """Stream mic audio, calling callback with each PCM chunk."""
    try:
        proc = subprocess.Popen(["parec", "--rate=16000", "--channels=1",
                                "--format=s16le", "--record"],
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    except:
        return
    chunk_size = int(SAMPLE_RATE * chunk_secs) * 2  # 16-bit = 2 bytes
    try:
        while True:
            data = proc.stdout.read(chunk_size)
            if not data or len(data) < chunk_size:
                break
            callback_chunk(data)
    except:
        proc.kill()

# ============================================================
# STT via moonshine
# ============================================================

def transcribe(pcm_bytes):
    """Transcribe PCM audio, return text."""
    stt_script = os.path.join(SCRIPT_DIR, "aleph-stt.py")
    if not os.path.exists(stt_script):
        # Check common paths
        for p in ["/usr/local/lib/aleph/aleph-stt.py",
                  os.path.expanduser("~/.local/bin/aleph-stt.py")]:
            if os.path.exists(p):
                stt_script = p
                break
    try:
        proc = subprocess.run(["python3", stt_script],
                             input=pcm_bytes, capture_output=True, timeout=30)
        data = json.loads(proc.stdout)
        return data.get("text", "").strip().lower()
    except:
        return ""

# ============================================================
# TTS via espeak-ng
# ============================================================

def speak(text):
    """Speak text, streaming sentence by sentence."""
    sentences = [s.strip() for s in text.replace("!", ".").replace("?", ".").split(".")
                 if s.strip()]
    for sentence in sentences:
        try:
            subprocess.run(["espeak-ng", "-v", "en-us", "-s", "155", "-p", "50",
                          "--stdin"], input=sentence.encode(), timeout=30,
                          stderr=subprocess.DEVNULL)
            time.sleep(0.05)
        except:
            try:
                subprocess.run(["espeak", "-v", "en-us", "-s", "155",
                              "--stdin"], input=sentence.encode(), timeout=30,
                              stderr=subprocess.DEVNULL)
            except:
                print(f"  {sentence}")

def play_beep():
    """Play a short beep to indicate listening mode."""
    try:
        dur = 0.15
        rate = 22050
        t = np.linspace(0, dur, int(rate * dur), False)
        tone = (np.sin(2 * np.pi * 880 * t) * 0.3).astype(np.float32)
        pcm = (tone * 32767).astype(np.int16).tobytes()
        proc = subprocess.Popen(["aplay", "-q", "-f", "S16_LE", "-r", str(rate),
                                "-c", "1"], stdin=subprocess.PIPE,
                               stderr=subprocess.DEVNULL)
        proc.stdin.write(pcm)
        proc.stdin.close()
        proc.wait()
    except:
        print("\n  [beep]", end=" ", flush=True)

# ============================================================
# Wake word detection
# ============================================================

def audio_energy(data):
    """Compute RMS energy of PCM data."""
    samples = np.frombuffer(data, dtype=np.int16).astype(np.float32)
    return np.sqrt(np.mean(samples ** 2)) if len(samples) > 0 else 0

class WakeWordDetector:
    """Detects 'aleph' wake word from mic stream."""
    
    def __init__(self):
        self.buffer = b""
        self.energy_threshold = 500  # Adjust based on mic sensitivity
        self.wake_detected = False
        self.last_query = ""
    
    def process_chunk(self, data):
        """Process a PCM chunk. Returns True if wake word detected."""
        energy = audio_energy(data)
        
        if energy < self.energy_threshold:
            self.buffer = b""
            return False
        
        self.buffer += data
        
        # When we have ~2 seconds of audio with energy, transcribe
        if len(self.buffer) >= SAMPLE_RATE * 2 * 2:  # 2s of 16-bit PCM
            text = transcribe(self.buffer)
            self.buffer = b""
            
            if text:
                # Check for wake words
                for wake in WAKE_WORDS:
                    if wake in text:
                        print(f"\n  🔔 Wake word detected: \"{text[:50]}\"")
                        self.wake_detected = True
                        return True
        return False

# ============================================================
# Query flow
# ============================================================

def ask_aleph(query):
    """Send query to Aleph API, return answer."""
    import urllib.request
    body = json.dumps({"question": query, "top_k": 5}).encode()
    req = urllib.request.Request(f"{API_BASE}/api/ask", data=body,
                                headers={"Content-Type": "application/json"})
    try:
        resp = urllib.request.urlopen(req, timeout=15)
        data = json.loads(resp.read())
        return data.get("answer", "No answer")
    except Exception as e:
        return f"Error: {e}"

def do_query():
    """Record a query, process it, speak the answer."""
    play_beep()
    print("\n  🎤 Recording query (5s)...", end=" ", flush=True)
    
    audio = record_seconds(5)
    if len(audio) < 8000:
        print("No audio detected.")
        return
    
    print("Transcribing...", end=" ", flush=True)
    query = transcribe(audio)
    
    if not query:
        print("No speech recognized.")
        return
    
    print(f"You: {query}")
    print("Thinking...", end=" ", flush=True)
    
    answer = ask_aleph(query)
    print(f"Aleph: {answer[:100]}{'...' if len(answer)>100 else ''}")
    
    print("Speaking...", end=" ", flush=True)
    speak(answer)
    print(" Done")

# ============================================================
# Main loop
# ============================================================

def wake_word_loop():
    """Background loop: detect wake word → query → loop."""
    print("  Aleph Voice Assistant — Listening for wake word...")
    print("  Say \"Aleph\" or \"Okay Aleph\" to start")
    print("  Press Ctrl+C to exit\n")
    
    detector = WakeWordDetector()
    
    def chunk_callback(data):
        if detector.process_chunk(data):
            # Wake word detected — do query in main thread
            pass
    
    # Start streaming in a background thread
    stream_thread = threading.Thread(target=record_streaming, args=(chunk_callback,), daemon=True)
    stream_thread.start()
    
    try:
        while True:
            if detector.wake_detected:
                detector.wake_detected = False
                do_query()
                print("\n  Listening for wake word...")
            time.sleep(0.1)
    except KeyboardInterrupt:
        print("\n  Goodbye!")

def single_query():
    """Single query mode (for aleph listen)."""
    print("  🎤 Recording (5s)...", end=" ", flush=True)
    audio = record_seconds(5)
    if len(audio) < 8000:
        print("No audio.")
        # Fallback to stdin
        query = input("\n  Type: ").strip()
        if query:
            answer = ask_aleph(query)
            print(f"  Aleph: {answer}")
            speak(answer)
        return
    print("Transcribing...", end=" ", flush=True)
    query = transcribe(audio)
    if query:
        print(f"\n  You: {query}")
        answer = ask_aleph(query)
        print(f"  Aleph: {answer}")
        speak(answer)
    else:
        print("No speech.")

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--once", action="store_true", help="Single query mode")
    parser.add_argument("--text", help="Process text directly")
    args = parser.parse_args()
    
    if args.text:
        answer = ask_aleph(args.text)
        print(answer)
        speak(answer)
    elif args.once:
        single_query()
    else:
        wake_word_loop()
