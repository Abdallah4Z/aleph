#!/usr/bin/env python3
"""Aleph Voice Assistant — wake word + query loop with desktop popups.

Wake words: "jarvis", "hey jarvis", "hi jarvis", "aleph", "okay aleph", "hey aleph"
Sends desktop notifications that stay until the query completes.
Plays a chime on activation. Speaks the response via espeak-ng.
"""
import sys, os, json, time, struct, argparse, subprocess, threading, shutil, re
import numpy as np

SAMPLE_RATE = 16000
WAKE_WORDS = ["jarvis", "hey jarvis", "hi jarvis", "aleph", "okay aleph", "hey aleph"]
API_BASE = "http://127.0.0.1:2198"
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))

# ============================================================
# Desktop notification
# ============================================================

NOTIF_ID = 9999

def notif(msg, urgency="normal", expire=0):
    """Send a desktop notification that persists (expire=0 = until dismissed)."""
    urgency_map = {"low": "low", "normal": "normal", "critical": "critical"}
    try:
        subprocess.run([
            "notify-send",
            "-u", urgency_map.get(urgency, "normal"),
            "-t", str(expire * 1000 if expire > 0 else 0),
            "-r", str(NOTIF_ID),
            "-a", "Aleph",
            "Aleph Voice",
            msg
        ], timeout=2, stderr=subprocess.DEVNULL)
    except:
        pass

def notif_clear():
    """Dismiss the notification."""
    try:
        subprocess.run(["notify-send", "-r", str(NOTIF_ID), "-t", "1", "-a", "Aleph", "", ""],
                      timeout=1, stderr=subprocess.DEVNULL)
    except:
        pass

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
    chunk_size = int(SAMPLE_RATE * chunk_secs) * 2
    try:
        while True:
            data = proc.stdout.read(chunk_size)
            if not data or len(data) < chunk_size:
                break
            callback_chunk(data)
    except:
        proc.kill()

# ============================================================
# Audio cues
# ============================================================

def play_tone(freq=880, duration=0.15, amplitude=0.3):
    """Play a sine wave tone through aplay."""
    try:
        rate = 22050
        t = np.linspace(0, duration, int(rate * duration), False)
        tone = (np.sin(2 * np.pi * freq * t) * amplitude).astype(np.float32)
        pcm = (tone * 32767).astype(np.int16).tobytes()
        proc = subprocess.Popen(["aplay", "-q", "-f", "S16_LE", "-r", str(rate),
                                "-c", "1"], stdin=subprocess.PIPE,
                               stderr=subprocess.DEVNULL)
        proc.stdin.write(pcm)
        proc.stdin.close()
        proc.wait()
    except:
        pass

def play_activation_chime():
    """Two-tone ascending chime."""
    play_tone(660, 0.1, 0.2)
    time.sleep(0.05)
    play_tone(880, 0.15, 0.25)

def play_done_chime():
    """Single short tone."""
    play_tone(440, 0.1, 0.15)

# ============================================================
# STT via moonshine
# ============================================================

def transcribe(pcm_bytes):
    """Transcribe PCM audio, return text."""
    stt_script = os.path.join(SCRIPT_DIR, "aleph-stt.py")
    if not os.path.exists(stt_script):
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
                pass

# ============================================================
# Wake word detection
# ============================================================

def audio_energy(data):
    samples = np.frombuffer(data, dtype=np.int16).astype(np.float32)
    return np.sqrt(np.mean(samples ** 2)) if len(samples) > 0 else 0

class WakeWordDetector:
    def __init__(self):
        self.buffer = b""
        self.energy_threshold = 500
        self.wake_detected = False

    def process_chunk(self, data):
        energy = audio_energy(data)
        if energy < self.energy_threshold:
            self.buffer = b""
            return False
        self.buffer += data
        if len(self.buffer) >= SAMPLE_RATE * 2 * 2:
            text = transcribe(self.buffer)
            self.buffer = b""
            if text:
                for wake in WAKE_WORDS:
                    if wake in text:
                        self.wake_detected = True
                        return True
        return False

# ============================================================
# Query flow
# ============================================================

def ask_aleph(query):
    """Send query to Aleph API, return answer."""
    import urllib.request
    body = json.dumps({"question": query, "top_k": 8}).encode()
    req = urllib.request.Request(f"{API_BASE}/api/ask", data=body,
                                headers={"Content-Type": "application/json"})
    try:
        resp = urllib.request.urlopen(req, timeout=15)
        data = json.loads(resp.read())
        return data.get("answer", "No answer")
    except Exception as e:
        return f"Error: {e}"

def do_query():
    """Record query, process, speak answer. Desktop popup stays throughout."""
    notif("🎤 Listening... speak now")
    play_activation_chime()

    audio = record_seconds(5)
    if len(audio) < 8000:
        notif("No audio detected", urgency="low")
        time.sleep(2)
        notif_clear()
        return

    notif("🔄 Transcribing...")
    query = transcribe(audio)

    if not query:
        notif("Could not understand speech. Try again.", urgency="low")
        time.sleep(2)
        notif_clear()
        return

    notif(f"🤔 Thinking...")
    answer = ask_aleph(query)

    # Update popup with answer
    preview = answer[:120] + ("..." if len(answer) > 120 else "")
    notif(f"💬 {preview}")

    speak(answer)
    play_done_chime()
    time.sleep(0.5)
    notif_clear()

# ============================================================
# Main loop
# ============================================================

def wake_word_loop():
    """Background loop: detect wake word → query → loop."""
    notif("🎧 Listening for 'Jarvis' or 'Aleph'", urgency="low")
    
    detector = WakeWordDetector()
    
    def chunk_callback(data):
        if detector.process_chunk(data):
            pass
    
    stream_thread = threading.Thread(target=record_streaming, args=(chunk_callback,), daemon=True)
    stream_thread.start()
    
    try:
        while True:
            if detector.wake_detected:
                detector.wake_detected = False
                do_query()
                notif("🎧 Listening for 'Jarvis' or 'Aleph'", urgency="low")
            time.sleep(0.1)
    except KeyboardInterrupt:
        notif("Voice assistant stopped.", urgency="low")
        notif_clear()

def single_query():
    """Single query mode."""
    notif("🎤 Recording (5s)... speak now")
    play_activation_chime()
    audio = record_seconds(5)
    if len(audio) < 8000:
        notif_clear()
        query = input("\n  Type: ").strip()
        if query:
            answer = ask_aleph(query)
            print(f"  {answer}")
            speak(answer)
        return
    query = transcribe(audio)
    if query:
        notif("🤔 Thinking...")
        answer = ask_aleph(query)
        print(f"  {answer}")
        speak(answer)
    notif_clear()

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
