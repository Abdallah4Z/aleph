use anyhow::Result;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

const API_BASE: &str = "http://127.0.0.1:2198";

fn script_path(name: &str) -> String {
    let myself = std::env::current_exe().ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let candidates = vec![
        myself.join(name),
        std::path::PathBuf::from("/usr/local/lib/aleph").join(name),
        std::path::PathBuf::from("/home/abdallah/Aleph/scripts").join(name),
    ];
    for p in &candidates {
        if p.exists() { return p.to_string_lossy().to_string(); }
    }
    name.into()
}

/// Transcribe audio via moonshine-tiny Python script.
fn stt(audio_pcm: &[u8]) -> Result<String> {
    let mut child = Command::new("python3")
        .arg(&script_path("aleph-stt.py"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    child.stdin.take().unwrap().write_all(audio_pcm)?;
    let output = child.wait_with_output()?;
    let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    Ok(data["text"].as_str().unwrap_or("").to_string())
}

/// Speak text via espeak-ng (streaming, natural).
fn tts(text: &str) -> Result<()> {
    // Split into sentences for streaming delivery
    let sentences: Vec<&str> = text.split(|c: char| c == '.' || c == '!' || c == '?')
        .map(|s| s.trim()).filter(|s| !s.is_empty()).collect();

    for sentence in &sentences {
        // Write each sentence to espeak-ng via stdin pipe (streaming)
        let child = Command::new("espeak-ng")
            .args(["-v", "en-us", "-s", "155", "-p", "50", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();

        match child {
            Ok(mut c) => {
                writeln!(c.stdin.take().unwrap(), "{}", sentence)?;
                let _ = c.wait();
                std::thread::sleep(Duration::from_millis(50)); // small gap between sentences
            }
            Err(_) => {
                // Fallback to espeak
                if let Ok(mut c) = Command::new("espeak")
                    .args(["-v", "en-us", "-s", "155", "--stdin"])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    writeln!(c.stdin.take().unwrap(), "{}", sentence)?;
                    let _ = c.wait();
                } else {
                    // Print fallback
                    println!("  {}", sentence);
                }
            }
        }
    }
    Ok(())
}

/// Record N seconds of audio from mic, return raw 16kHz PCM.
fn record(duration_secs: u64) -> Result<Vec<u8>> {
    let tmp = std::env::temp_dir().join("aleph_query.wav");

    // arecord → WAV file, strip header
    if let Ok(status) = Command::new("arecord")
        .args(["-q", "-f", "S16_LE", "-r", "16000", "-c", "1",
               "-d", &duration_secs.to_string(), tmp.to_str().unwrap()])
        .status()
    {
        if status.success() {
            let wav = std::fs::read(&tmp)?;
            let _ = std::fs::remove_file(&tmp);
            return Ok(if wav.len() > 44 { wav[44..].to_vec() } else { wav });
        }
    }

    // parec fallback
    if let Ok(output) = Command::new("parec")
        .args(["--rate=16000", "--channels=1", "--format=s16le",
               "--record", "--duration", &duration_secs.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        if !output.stdout.is_empty() { return Ok(output.stdout); }
    }

    anyhow::bail!("Install 'alsa-utils' (arecord) for mic capture")
}

fn ask(query: &str) -> Result<String> {
    let body = serde_json::json!({"question": query, "top_k": 5});
    let resp = ureq::post(&format!("{}/api/ask", API_BASE))
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| anyhow::anyhow!("API: {}", e))?;
    let body = resp.into_body().read_to_string()?;
    let data: serde_json::Value = serde_json::from_str(&body)?;
    Ok(data["answer"].as_str().unwrap_or("No answer").to_string())
}

pub fn run_listen_loop() -> Result<()> {
    println!("  ╔═══════════════════════════════════════╗");
    println!("  ║        Aleph Voice — Listening Mode    ║");
    println!("  ║  STT: moonshine-tiny  TTS: espeak-ng  ║");
    println!("  ╚═══════════════════════════════════════╝");
    println!();
    println!("  Speak after the beep. Press Ctrl+C to exit.");

    loop {
        print!("\n  🎤 Listening (5s)...");
        std::io::stdout().flush()?;

        let audio = match record(5) {
            Ok(a) if a.len() > 8000 => a,
            _ => {
                print!("\r  ⌨️  Type instead: ");
                std::io::stdout().flush()?;
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                let q = input.trim().to_string();
                if q.is_empty() { continue; }
                process(&q)?;
                continue;
            }
        };

        print!("\r  🔄 Transcribing...");
        std::io::stdout().flush()?;

        let query = match stt(&audio) {
            Ok(q) if !q.is_empty() => q,
            Ok(_) => { println!("\r  No speech detected."); continue; }
            Err(e) => {
                print!("\r  ✗ STT error ({}). Type: ", e);
                std::io::stdout().flush()?;
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                input.trim().to_string()
            }
        };

        process(&query)?;
    }
}

fn process(query: &str) -> Result<()> {
    println!("\r  You: \"{}\"", query);
    print!("  🤔 Thinking...");
    std::io::stdout().flush()?;

    let answer = match ask(query) {
        Ok(a) => a,
        Err(e) => { println!("\r  ✗ {}", e); return Ok(()); }
    };

    println!("\r  Aleph: {}", answer);
    print!("  🔈 Speaking...");
    std::io::stdout().flush()?;

    if let Err(e) = tts(&answer) {
        eprintln!("\r  ⚠ TTS: {}", e);
    }
    println!("\r  ✅ Done");

    Ok(())
}
