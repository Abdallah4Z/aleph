pub struct CodeContext {
    pub file: Option<String>,
    pub project: Option<String>,
    pub branch: Option<String>,
}

fn split_title(title: &str) -> Vec<&str> {
    // Handle both " — " (em dash) and " - " (hyphen) separators
    if title.contains(" — ") {
        title.split(" — ").map(|s| s.trim()).collect()
    } else {
        title.split(" - ").map(|s| s.trim()).collect()
    }
}

/// Extract code context from a window title and app name.
pub fn parse_code_context(app: &str, title: &str) -> Option<CodeContext> {
    let al = app.to_lowercase();

    let is_editor = al.contains("code") || al.contains("zed") || al.contains("vim")
        || al.contains("neovim") || al.contains("intellij") || al.contains("pycharm")
        || al.contains("rustrover") || al.contains("goland") || al.contains("cursor")
        || al.contains("vscodium") || al.contains("emacs") || al.contains("sublime")
        || al.contains("atom") || al.contains("codium");

    if !is_editor {
        return None;
    }

    let mut ctx = CodeContext { file: None, project: None, branch: None };

    // VS Code / Cursor / VSCodium: "file.rs - Project - Visual Studio Code"
    if al.contains("code") || al.contains("cursor") || al.contains("vscodium") || al.contains("codium") {
        // Strip known editor suffixes from title
        let suffixes = ["visual studio code", "visual studio code - insiders", "cursor", "vscodium", "code - oss"];
        let mut body = title.to_string();
        let lower = title.to_lowercase();
        for suffix in &suffixes {
            if let Some(idx) = lower.rfind(suffix) {
                body = body[..idx].trim().to_string();
                break;
            }
        }

        let parts = split_title(&body);
        if parts.len() >= 2 {
            ctx.file = Some(parts[0].to_string());
            ctx.project = Some(parts[1].to_string());
        } else if parts.len() == 1 && !parts[0].is_empty() && parts[0] != title.trim() {
            ctx.file = Some(parts[0].to_string());
        }
    }

    // Zed: "file.rs - Zed"
    if al.contains("zed") {
        let parts = split_title(title);
        if parts.len() >= 2 {
            ctx.file = Some(parts[0].to_string());
            if parts.len() >= 3 {
                ctx.project = Some(parts[parts.len() - 2].to_string());
            }
        }
    }

    // Vim/Neovim: "file.rs - NVIM"
    if al.contains("vim") || al.contains("neovim") {
        for sep in &[" - ", " — "] {
            if let Some(stripped) = title.rsplitn(2, sep).nth(1) {
                ctx.file = Some(stripped.trim().to_string());
                break;
            }
        }
    }

    // IntelliJ-based: "file.rs - Project - IntelliJ IDEA"
    if al.contains("intellij") || al.contains("pycharm") || al.contains("rustrover") || al.contains("goland") {
        let known = ["intellij idea", "pycharm", "rustrover", "goland", "idea", "webstorm", "clion"];
        let mut body = title.to_string();
        let lower = title.to_lowercase();
        for editor in &known {
            if let Some(idx) = lower.rfind(editor) {
                body = body[..idx].trim().to_string();
                break;
            }
        }
        let parts = split_title(&body);
        if parts.len() >= 1 {
            ctx.file = Some(parts[0].to_string());
            if parts.len() >= 2 {
                ctx.project = Some(parts[1].to_string());
            }
        }
    }

    // Extract git branch if present in the file name: "[main]" or "(main)"
    if let Some(file) = &ctx.file {
        let pairs = [("[", "]"), ("(", ")")];
        for (open, close) in &pairs {
            if let Some(start) = file.rfind(open) {
                if let Some(end) = file[start..].find(close) {
                    let branch = file[start+1..start+end].to_string();
                    if !branch.is_empty() && branch.len() < 60 {
                        ctx.branch = Some(branch);
                        ctx.file = Some(file[..start].trim().to_string());
                        break;
                    }
                }
            }
        }
    }

    // Clean up file paths
    if let Some(f) = &ctx.file {
        let cleaned = f.trim_start_matches('/').trim_start_matches('~').trim_start_matches('\\');
        if !cleaned.is_empty() && cleaned.len() < 200 {
            ctx.file = Some(cleaned.to_string());
        }
    }

    Some(ctx)
}
