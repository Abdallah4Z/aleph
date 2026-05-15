use anyhow::Result;
use std::process::Command;

/// Find the aleph-voice.py script in known locations.
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
    // If not found, try running from PATH
    "aleph-voice.py".into()
}

/// Run the voice assistant in wake word mode.
pub fn run_listen_loop() -> Result<()> {
    let script = voice_script_path();
    eprintln!("aleph: voice: using script {}", script);

    let status = Command::new("python3")
        .arg(&script)
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to start voice assistant: {}", e))?;

    if !status.success() {
        anyhow::bail!("Voice assistant exited with code {:?}", status.code());
    }
    Ok(())
}
