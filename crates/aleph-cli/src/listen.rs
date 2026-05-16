use anyhow::Result;
use std::process::Command;
use std::io::Write;

fn voice_script_path() -> String {
    let myself = std::env::current_exe().ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let candidates = vec![
        myself.join("aleph-voice.py"),
        std::path::PathBuf::from("/usr/local/lib/aleph/aleph-voice.py"),
        std::path::PathBuf::from("/home/abdallah/Aleph/scripts/aleph-voice.py"),
    ];
    for p in &candidates {
        if p.exists() { return p.to_string_lossy().to_string(); }
    }
    "aleph-voice.py".into()
}

fn mic_available() -> bool {
    Command::new("arecord").args(["-l"]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .status().map(|s| s.success()).unwrap_or(false)
}

pub fn run_listen_loop() -> Result<()> {
    let script = voice_script_path();
    eprintln!("aleph: voice: using script {}", script);

    if !mic_available() {
        eprintln!("aleph: voice: no microphone detected. Starting text-only mode.");
        eprintln!("aleph: voice: type your questions, press Enter.");
        println!();
        text_mode()?;
        return Ok(());
    }

    eprintln!("aleph: voice: microphone detected. Starting wake word detection.");
    eprintln!("aleph: voice: say \"Jarvis\" or \"Aleph\" to activate.");
    println!();

    let status = Command::new("python3")
        .arg(&script)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to start voice assistant: {}", e))?;

    if !status.success() {
        eprintln!("aleph: voice: assistant exited. Falling back to text mode.");
        text_mode()?;
    }
    Ok(())
}

fn text_mode() -> Result<()> {
    println!("  Aleph — Ask me anything about your desktop activity.");
    println!("  Type 'q' to quit.\n");

    loop {
        print!("  > ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let query = input.trim().to_string();
        if query.is_empty() { continue; }
        if query == "q" || query == "quit" || query == "exit" { break; }

        let body = serde_json::json!({"question": query, "top_k": 8});
        match ureq::post("http://127.0.0.1:2198/api/ask")
            .header("Content-Type", "application/json")
            .send_json(&body)
        {
            Ok(resp) => {
                let body = resp.into_body().read_to_string().unwrap_or_default();
                let data: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                let answer = data["answer"].as_str().unwrap_or("No answer");
                println!("  {}", answer);
            }
            Err(e) => {
                eprintln!("  Error: {}", e);
            }
        }
    }
    Ok(())
}
