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

def log(msg):
    """Status line that stays visible."""
    sys.stderr.write(f"  {msg}\n")
    sys.stderr.flush()

# ============================================================
# Desktop notification
# ============================================================

NOTIF_ID = 9999

def notif(msg, urgency="normal", expire=0):
    try:
        subprocess.run([
            "notify-send", "-u", urgency, "-t", str(expire * 1000 if expire > 0 else 0),
            "-r", str(NOTIF_ID), "-a", "Aleph", "Aleph Voice", msg
        ], timeout=2, stderr=subprocess.DEVNULL)
    except:
        pass

def notif_clear():
    try:
        subprocess.run(["notify-send", "-r", str(NOTIF_ID), "-t", "1", "-a", "Aleph", "", ""],
                      timeout=1, stderr=subprocess.DEVNULL)
    except:
        pass

# ============================================================
# Audio capture
# ============================================================

def record_seconds(duration, rate=SAMPLE_RATE):
    tmp = f"/tmp/aleph_voice_{int(time.time())}.wav"
    for cmd, args in [
        (["arecord", "-q", "-f", "S16_LE", "-r", str(rate), "-c", "1", "-d", str(duration), tmp], True),
        (["parec", "--rate=16000", "--channels=1", "--format=s16le", "--record", "--duration", str(duration)], False),
    ]:
        try:
            if cmd[0] == "arecord":
                subprocess.run(cmd, timeout=duration + 5, stderr=subprocess.DEVNULL)
                if os.path.exists(tmp):
                    with open(tmp, "rb") as f:
                        d = f.read()
                    os.remove(tmp)
                    return d[44:] if len(d) > 44 else d
            else:
                result = subprocess.run(cmd, capture_output=True, timeout=duration + 5)
                if result.stdout and len(result.stdout) > 8000:
                    return result.stdout
        except:
            continue
    return b""

def record_streaming(callback, chunk_secs=0.5):
    """Stream mic audio via parec."""
    chunk = int(SAMPLE_RATE * chunk_secs) * 2
    try:
        proc = subprocess.Popen(["parec", "--rate=16000", "--channels=1", "--format=s16le", "--record"],
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    except:
        return
    try:
        while True:
            data = proc.stdout.read(chunk)
            if not data or len(data) < chunk:
                break
            callback(data)
    except:
        proc.kill()

# ============================================================
# Audio cues
# ============================================================

def play_tone(freq=880, duration=0.15, amp=0.3):
    try:
        rate = 22050
        t = np.linspace(0, duration, int(rate * duration), False)
        tone = (np.sin(2 * np.pi * freq * t) * amp).astype(np.float32)
        pcm = (tone * 32767).astype(np.int16).tobytes()
        proc = subprocess.Popen(["aplay", "-q", "-f", "S16_LE", "-r", str(rate), "-c", "1"],
                               stdin=subprocess.PIPE, stderr=subprocess.DEVNULL)
        proc.stdin.write(pcm)
        proc.stdin.close()
        proc.wait()
    except:
        pass

def play_chime():
    play_tone(660, 0.1, 0.2); time.sleep(0.05); play_tone(880, 0.15, 0.25)

def play_done():
    play_tone(440, 0.1, 0.15)

# ============================================================
# STT
# ============================================================

def transcribe(pcm_bytes):
    stt = os.path.join(SCRIPT_DIR, "aleph-stt.py")
    if not os.path.exists(stt):
        for p in ["/usr/local/lib/aleph/aleph-stt.py", os.path.expanduser("~/.local/bin/aleph-stt.py")]:
            if os.path.exists(p): stt = p; break
    try:
        proc = subprocess.run(["python3", stt], input=pcm_bytes, capture_output=True, timeout=30)
        data = json.loads(proc.stdout)
        return data.get("text", "").strip().lower()
    except Exception as e:
        log(f"STT error: {e}")
        return ""

# ============================================================
# TTS
# ============================================================

def speak(text):
    """Speak text using Kitten TTS."""
    tts = os.path.join(SCRIPT_DIR, "aleph-tts.py")
    if not os.path.exists(tts):
        # Fallback to espeak
        for s in [s.strip() for s in text.replace("!", ".").replace("?", ".").split(".") if s.strip()]:
            for cmd in [["espeak-ng", "-v", "en-us", "-s", "155", "-p", "50", "--stdin"],
                         ["espeak", "-v", "en-us", "-s", "155", "--stdin"]]:
                try:
                    subprocess.run(cmd, input=s.encode(), timeout=30, stderr=subprocess.DEVNULL)
                    time.sleep(0.05)
                    break
                except:
                    continue
        return
    try:
        proc = subprocess.Popen(["python3", tts, "--text", text, "--stream"],
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        # Read length-prefixed PCM chunks and play via aplay
        player = subprocess.Popen(["aplay", "-q", "-f", "S16_LE", "-r", "24000", "-c", "1"],
                                 stdin=subprocess.PIPE, stderr=subprocess.DEVNULL)
        while True:
            header = proc.stdout.read(4)
            if not header or len(header) < 4:
                break
            n_samples = struct.unpack('<I', header)[0]
            pcm = proc.stdout.read(n_samples * 2)  # 16-bit
            if not pcm:
                break
            player.stdin.write(pcm)
        player.stdin.close()
        player.wait()
    except:
        pass

# ============================================================
# Wake word detection
# ============================================================

def audio_energy(data):
    s = np.frombuffer(data, dtype=np.int16).astype(np.float32)
    return float(np.sqrt(np.mean(s ** 2))) if len(s) > 0 else 0

class WakeWordDetector:
    def __init__(self):
        self.buffer = b""
        self.threshold = 500
        self.detected = False

    def process(self, data):
        if audio_energy(data) < self.threshold:
            self.buffer = b""
            return False
        self.buffer += data
        if len(self.buffer) >= SAMPLE_RATE * 4:  # 4s buffer
            text = transcribe(self.buffer)
            self.buffer = b""
            if text and any(w in text for w in WAKE_WORDS):
                self.detected = True
                return True
        return False

# ============================================================
# Query
# ============================================================

def ask_aleph(query):
    import urllib.request
    body = json.dumps({"question": query, "top_k": 8}).encode()
    req = urllib.request.Request(f"{API_BASE}/api/ask", data=body, headers={"Content-Type": "application/json"})
    try:
        resp = urllib.request.urlopen(req, timeout=15)
        return json.loads(resp.read()).get("answer", "No answer")
    except Exception as e:
        return f"Error: {e}"

def do_query():
    notif("🎤 Listening...")
    play_chime()
    audio = record_seconds(5)
    if len(audio) < 8000:
        notif("No audio detected", urgency="low")
        time.sleep(1.5)
        notif_clear()
        return
    notif("🔄 Transcribing...")
    query = transcribe(audio)
    if not query:
        notif("Could not understand speech", urgency="low")
        time.sleep(1.5)
        notif_clear()
        return
    notif(f"🤔 Thinking...")
    answer = ask_aleph(query)
    preview = answer[:150] + ("..." if len(answer) > 150 else "")
    notif(f"💬 {preview}")
    speak(answer)
    play_done()
    time.sleep(0.5)
    notif_clear()

# ============================================================
# Main
# ============================================================

def wake_word_loop():
    log("Listening for 'Jarvis' or 'Aleph'...")
    notif("Say 'Jarvis' or 'Aleph'", urgency="low")
    detector = WakeWordDetector()
    def cb(data):
        if detector.process(data):
            pass
    t = threading.Thread(target=record_streaming, args=(cb,), daemon=True)
    t.start()
    try:
        while True:
            if detector.detected:
                detector.detected = False
                do_query()
                notif("Say 'Jarvis' or 'Aleph'", urgency="low")
            time.sleep(0.1)
    except KeyboardInterrupt:
        notif_clear()
        log("Stopped.")

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--once", action="store_true")
    parser.add_argument("--text")
    args = parser.parse_args()
    if args.text:
        answer = ask_aleph(args.text)
        print(answer)
        speak(answer)
    elif args.once:
        notif("🎤 Recording (5s)...")
        play_chime()
        audio = record_seconds(5)
        query = transcribe(audio) if len(audio) >= 8000 else ""
        if query:
            answer = ask_aleph(query)
            print(answer)
            speak(answer)
        notif_clear()
    else:
        wake_word_loop()
