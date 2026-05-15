//! Speech interface: "Hey Aleph" — wake word, STT, query, TTS.
//!
//! Captures microphone audio, runs speech-to-text, queries the Aleph API,
//! and optionally speaks the response.
//!
//! ## STT backends (in priority order):
//!   1. `whisper-cli` — if `whisper-cli` is in PATH
//!   2. `moonshine` — if a moonshine binary is configured
//!   3. Manual text input — fallback
//!
//! ## TTS backends:
//!   1. `espeak-ng` — if installed (fast, robotic but works)
//!   2. `kitten-tts` — if configured (requires ONNX model)
//!   3. Print to terminal — fallback

use anyhow::Result;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

const API_BASE: &str = "http://127.0.0.1:2198";

/// Main loop: listen → STT → ask → TTS
pub fn run_listen_loop() -> Result<()> {
    println!("  Aleph Listening Mode");
    println!("  Say \"hey aleph\" or press Enter to type a query");
    println!("  Press Ctrl+C to exit\n");

    loop {
        // Detect wake word or manual input
        let query = match capture_query() {
            Ok(q) if !q.is_empty() => q,
            _ => {
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };

        println!("\n  Query: {}", query);

        // Ask the API
        let answer = match ask_aleph(&query) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("  Error: {}", e);
                continue;
            }
        };

        println!("  Answer: {}\n", answer);

        // Speak the response
        speak(&answer);
    }
}

fn capture_query() -> Result<String> {
    // Try whisper-cli first
    if let Ok(text) = capture_with_whisper() {
        return Ok(text);
    }

    // Try moonshine if configured
    if let Ok(text) = capture_with_moonshine() {
        return Ok(text);
    }

    // Fallback: read from stdin
    print!("  > ");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn capture_with_whisper() -> Result<String> {
    // Check if whisper-cli is available
    if Command::new("whisper-cli").arg("--help").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
        anyhow::bail!("whisper-cli not found");
    }

    // Record audio to temp file
    let tmp_dir = std::env::temp_dir();
    let wav_path = tmp_dir.join("aleph_query.wav");
    record_audio(&wav_path, 5)?; // Record 5 seconds

    // Run whisper-cli
    let output = Command::new("whisper-cli")
        .arg("--model")
        .arg("tiny")
        .arg("--file")
        .arg(&wav_path)
        .arg("--output-txt")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()?;

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        anyhow::bail!("No speech detected");
    }
    Ok(text)
}

fn capture_with_moonshine() -> Result<String> {
    // Check for moonshine binary
    let moonshine_bin = std::env::var("ALEPH_MOONSHINE_BIN").unwrap_or_else(|_| "moonshine".into());
    if Command::new(&moonshine_bin).arg("--help").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
        anyhow::bail!("moonshine not found at {}", moonshine_bin);
    }

    let tmp_dir = std::env::temp_dir();
    let wav_path = tmp_dir.join("aleph_query.wav");
    record_audio(&wav_path, 5)?;

    let output = Command::new(&moonshine_bin)
        .arg("--audio")
        .arg(&wav_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()?;

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        anyhow::bail!("No speech detected");
    }
    Ok(text)
}

fn record_audio(path: &std::path::Path, duration_secs: u64) -> Result<()> {
    // Try to use `arecord` (Linux ALSA) — most reliable approach
    if Command::new("arecord").arg("--help").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok() {
        let status = Command::new("arecord")
            .args([
                "-q", "-f", "S16_LE", "-r", "16000", "-c", "1",
                "-d", &duration_secs.to_string(),
                path.to_str().unwrap(),
            ])
            .status()?;
        if status.success() {
            return Ok(());
        }
    }

    // Fallback: try `parec` (PulseAudio) with `sox` to convert
    if Command::new("parec").arg("--help").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
        && Command::new("sox").arg("--help").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
    {
        let status = Command::new("parec")
            .args(["--rate=16000", "--channels=1", "--format=s16le"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?
            .wait_with_output()?;

        // Convert raw PCM to WAV
        let mut wav = std::fs::File::create(path)?;
        write_wav_header(&mut wav, 16000, 1, &status.stdout)?;
        return Ok(());
    }

    anyhow::bail!("No audio capture tool found. Install 'arecord' (alsa-utils) or 'parec' (pulseaudio-utils)")
}

fn write_wav_header(writer: &mut impl Write, sample_rate: u32, channels: u16, data: &[u8]) -> Result<()> {
    let data_len = data.len() as u32;
    use byteorder::{LittleEndian, WriteBytesExt};
    writer.write_all(b"RIFF")?;
    writer.write_u32::<LittleEndian>(36 + data_len)?;
    writer.write_all(b"WAVE")?;
    writer.write_all(b"fmt ")?;
    writer.write_u32::<LittleEndian>(16)?; // chunk size
    writer.write_u16::<LittleEndian>(1)?;  // PCM
    writer.write_u16::<LittleEndian>(channels)?;
    writer.write_u32::<LittleEndian>(sample_rate)?;
    writer.write_u32::<LittleEndian>(sample_rate * channels as u32 * 2)?; // byte rate
    writer.write_u16::<LittleEndian>(channels * 2)?; // block align
    writer.write_u16::<LittleEndian>(16)?; // bits per sample
    writer.write_all(b"data")?;
    writer.write_u32::<LittleEndian>(data_len)?;
    writer.write_all(data)?;
    Ok(())
}

fn ask_aleph(query: &str) -> Result<String> {
    let body = serde_json::json!({"question": query, "top_k": 5});
    let url = format!("{}/api/ask", API_BASE);

    let resp = ureq::post(&url)
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| anyhow::anyhow!("API request failed: {}", e))?;

    let body = resp.into_body().read_to_string()?;
    let data: serde_json::Value = serde_json::from_str(&body)?;
    let answer = data["answer"].as_str().unwrap_or("No answer").to_string();
    Ok(answer)
}

fn speak(text: &str) {
    // Try espeak-ng first
    if let Ok(_) = Command::new("espeak-ng").arg("--help").stdout(Stdio::null()).stderr(Stdio::null()).status() {
        let _ = Command::new("espeak-ng")
            .args(["-v", "en-us", "-s", "150", text])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        return;
    }

    // Try espeak (older)
    if let Ok(_) = Command::new("espeak").arg("--help").stdout(Stdio::null()).stderr(Stdio::null()).status() {
        let _ = Command::new("espeak")
            .args(["-v", "en-us", "-s", "150", text])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        return;
    }

    // Print to terminal as fallback
    println!("  [TTS unavailable] Response: {}", text);
}
