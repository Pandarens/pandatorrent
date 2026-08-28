//! Turning a finished download into a Steam-like library card: a readable
//! title, cached artwork, and something to launch.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// How deep to walk an install folder looking for executables.
const MAX_SCAN_DEPTH: usize = 4;
/// Guard against pathological trees; a game never has this many binaries.
const MAX_EXECUTABLES: usize = 400;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutableCandidate {
    pub path: String,
    pub file_name: String,
    pub size_bytes: u64,
    /// Depth below the install root; shallower binaries are usually the game.
    pub depth: usize,
    /// `setup.exe` and friends — the user runs these once, to install.
    pub is_installer: bool,
    /// Higher is a better guess for "the thing to launch".
    pub score: i32,
}

/// Filenames that are never the game itself.
const NOISE: &[&str] = &[
    "unins",
    "uninstall",
    "vcredist",
    "vc_redist",
    "dxsetup",
    "directx",
    "dotnet",
    "oalinst",
    "redist",
    "crashreport",
    "crashhandler",
    "crashsender",
    "ue4prereqsetup",
    "ueprereqsetup",
    "epicgameslauncher",
    "quicksfv",
    "7z",
    "activation",
];

const INSTALLER_NAMES: &[&str] = &["setup", "install", "autorun", "start"];

/// Strips the release-group noise RuTracker titles are full of.
///
/// `Half-Life 2 [P] [RUS + ENG] (2004, FPS) [Repack] от R.G. Механики`
/// becomes `Half-Life 2`.
pub fn clean_title(raw: &str) -> String {
    let raw = raw.trim();

    // Everything from the first bracket onwards is metadata, as long as there
    // is a real name in front of it.
    let cut = raw
        .char_indices()
        .find(|(_, c)| matches!(c, '[' | '('))
        .map(|(i, _)| i)
        .filter(|i| *i > 2)
        .unwrap_or(raw.len());

    let mut title = raw[..cut].trim().to_string();

    // Torrent names sometimes end with a dangling separator once the brackets
    // are gone.
    while title.ends_with(['-', '–', '—', '.', ',', '/', '|', ':']) {
        title.pop();
        title = title.trim_end().to_string();
    }

    title = title.split_whitespace().collect::<Vec<_>>().join(" ");

    if title.is_empty() {
        raw.split_whitespace().take(6).collect::<Vec<_>>().join(" ")
    } else {
        title
    }
}

/// Walks an install directory and ranks the executables it finds.
pub fn scan_executables(root: &Path, title: &str) -> AppResult<Vec<ExecutableCandidate>> {
    if !root.exists() {
        return Err(AppError::msg(format!(
            "папка не найдена: {}",
            root.display()
        )));
    }

    let mut found = Vec::new();
    walk(root, root, 0, &mut found)?;

    let title_words: Vec<String> = title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(str::to_string)
        .collect();

    for c in &mut found {
        c.score = score(c, &title_words);
    }
    found.sort_by(|a, b| b.score.cmp(&a.score).then(a.path.cmp(&b.path)));
    Ok(found)
}

fn walk(
    root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<ExecutableCandidate>,
) -> AppResult<()> {
    if depth > MAX_SCAN_DEPTH || out.len() >= MAX_EXECUTABLES {
        return Ok(());
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        // An unreadable subfolder should not abort the whole scan.
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };

        if meta.is_dir() {
            walk(root, &path, depth + 1, out)?;
            continue;
        }
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        {
            continue;
        }

        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let stem = file_name.to_lowercase();

        out.push(ExecutableCandidate {
            path: path.to_string_lossy().to_string(),
            is_installer: INSTALLER_NAMES.iter().any(|n| stem.starts_with(n)),
            file_name,
            size_bytes: meta.len(),
            depth,
            score: 0,
        });

        if out.len() >= MAX_EXECUTABLES {
            return Ok(());
        }
    }
    Ok(())
}

fn score(c: &ExecutableCandidate, title_words: &[String]) -> i32 {
    let name = c.file_name.to_lowercase();

    if NOISE.iter().any(|n| name.contains(n)) {
        return -100;
    }

    let mut score = 0;

    // Shallow binaries beat ones buried in engine subfolders.
    score += (MAX_SCAN_DEPTH as i32 - c.depth as i32) * 8;

    // A name matching the release title is the strongest signal available.
    if title_words.iter().any(|w| name.contains(w.as_str())) {
        score += 40;
    }

    // Common engine layouts.
    let lower_path = c.path.to_lowercase();
    if lower_path.contains("binaries\\win64") || lower_path.contains("bin\\x64") {
        score += 15;
    }

    // Big binaries are usually the game; launchers are small.
    score += match c.size_bytes {
        s if s > 100 * 1024 * 1024 => 20,
        s if s > 10 * 1024 * 1024 => 12,
        s if s > 1024 * 1024 => 5,
        _ => 0,
    };

    // An installer is worth surfacing, but never as the default launch target.
    if c.is_installer {
        score -= 25;
    }

    score
}

/// Launches an executable in its own directory, detached from the app.
pub fn launch(exe: &str) -> AppResult<()> {
    let path = PathBuf::from(exe);
    if !path.exists() {
        return Err(AppError::msg(format!("файл не найден: {exe}")));
    }
    let dir = path.parent().unwrap_or(Path::new("."));

    std::process::Command::new(&path)
        .current_dir(dir)
        .spawn()
        .map_err(|e| AppError::msg(format!("не удалось запустить {exe}: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_a_typical_rutracker_name() {
        assert_eq!(
            clean_title("Half-Life 2 [P] [RUS + ENG] (2004, FPS) [Repack] от R.G. Механики"),
            "Half-Life 2"
        );
    }

    #[test]
    fn keeps_a_name_that_has_no_metadata() {
        assert_eq!(clean_title("Portal 2"), "Portal 2");
    }

    #[test]
    fn trims_dangling_separators() {
        assert_eq!(clean_title("Cyberpunk 2077 - (2020)"), "Cyberpunk 2077");
    }

    #[test]
    fn does_not_eat_a_leading_bracket_name() {
        // The bracket is too early to be metadata, so nothing is cut.
        assert_eq!(clean_title("[SUB] Game"), "[SUB] Game");
    }

    #[test]
    fn ranks_the_game_above_the_uninstaller() {
        let game = ExecutableCandidate {
            path: "C:\\g\\Portal2.exe".into(),
            file_name: "Portal2.exe".into(),
            size_bytes: 200 * 1024 * 1024,
            depth: 0,
            is_installer: false,
            score: 0,
        };
        let uninst = ExecutableCandidate {
            path: "C:\\g\\unins000.exe".into(),
            file_name: "unins000.exe".into(),
            size_bytes: 900 * 1024,
            depth: 0,
            is_installer: false,
            score: 0,
        };
        let words = vec!["portal2".to_string()];
        assert!(score(&game, &words) > score(&uninst, &words));
        assert_eq!(score(&uninst, &words), -100);
    }

    #[test]
    fn installer_ranks_below_the_game() {
        let words = vec!["portal".to_string()];
        let setup = ExecutableCandidate {
            path: "C:\\g\\setup.exe".into(),
            file_name: "setup.exe".into(),
            size_bytes: 5 * 1024 * 1024,
            depth: 0,
            is_installer: true,
            score: 0,
        };
        let game = ExecutableCandidate {
            path: "C:\\g\\portal.exe".into(),
            file_name: "portal.exe".into(),
            size_bytes: 5 * 1024 * 1024,
            depth: 0,
            is_installer: false,
            score: 0,
        };
        assert!(score(&game, &words) > score(&setup, &words));
    }
}
