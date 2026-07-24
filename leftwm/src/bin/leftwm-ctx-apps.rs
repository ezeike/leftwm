// NEW — leftwm/src/bin/leftwm-ctx-apps.rs

use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Extracts the first whitespace-delimited word of a shell command,
/// lowercased — mirrors xmonad's `spawnIfNotOnWorkspace`'s `exeName`
/// derivation (`takeWhile (/= ' ')` + `toLower`,
/// ~/config/wm/xmonad/Workspaces.hs).
fn exe_name(command: &str) -> String {
    command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase()
}

/// True if `app`'s exe name appears as a case-insensitive substring of any
/// WM_CLASS string in `running_classes` — mirrors xmonad's `isInfixOf`
/// dedup check. An empty exe name (blank `app`) never matches, so a
/// misconfigured empty command is treated as "not running" without
/// falsely matching every class.
fn is_app_running(app: &str, running_classes: &[String]) -> bool {
    let needle = exe_name(app);
    if needle.is_empty() {
        return false;
    }
    running_classes
        .iter()
        .any(|class| class.to_lowercase().contains(&needle))
}

/// Parses `wmctrl -d` output, returning the desktop index whose name
/// (the last whitespace-delimited field on the line) matches `tag_name`
/// exactly.
///
/// Known limitation: only the trailing whitespace-delimited token is
/// compared, so a desktop/tag name containing an embedded space would
/// never match. Real tag names in this repo are always hyphenated single
/// tokens (e.g. `docker-docker`), so this isn't hit in practice.
fn find_desktop_index(wmctrl_d_output: &str, tag_name: &str) -> Option<u32> {
    for line in wmctrl_d_output.lines() {
        let index: u32 = match line.split_whitespace().next().and_then(|s| s.parse().ok()) {
            Some(i) => i,
            None => continue,
        };
        if line.split_whitespace().last() == Some(tag_name) {
            return Some(index);
        }
    }
    None
}

/// Parses `wmctrl -lx` output into `(desktop_index, wm_class)` pairs.
/// Columns are `<window-id> <desktop-index> <WM_CLASS> ...`; WM_CLASS
/// itself may contain dots (e.g. `org.wezfurlong.wezterm.org...`), so it
/// is taken as the whole third whitespace-delimited field, not split
/// further.
fn parse_wmctrl_lx(output: &str) -> Vec<(u32, String)> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let _window_id = parts.next()?;
            let desktop: u32 = parts.next()?.parse().ok()?;
            let class = parts.next()?.to_string();
            Some((desktop, class))
        })
        .collect()
}

/// Hardcoded intentionally, matching `leftwm/src/config.rs`'s
/// `CTX_CONFIG_PATH` — this fork is personal, not meant to be portable to
/// another machine's directory layout. Overridable via
/// `LEFTWM_CTX_CONFIG_PATH` so end-to-end tests can point at a fixture
/// file without touching the real ctx/config.yaml.
const CTX_CONFIG_PATH: &str = "/home/enielson/config/ctx/config.yaml";

fn ctx_config_path() -> PathBuf {
    env::var("LEFTWM_CTX_CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(CTX_CONFIG_PATH))
}

fn run_wmctrl(args: &[&str]) -> String {
    match Command::new("wmctrl").args(args).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(e) => {
            eprintln!("leftwm-ctx-apps: failed to run wmctrl {args:?}: {e}");
            String::new()
        }
    }
}

fn main() {
    let Some(tag_name) = env::args().nth(1) else {
        eprintln!("usage: leftwm-ctx-apps <tag-name>");
        std::process::exit(1);
    };

    let apps = match leftwm::ctx_config::tag_apps(&ctx_config_path()) {
        Ok(map) => map.get(&tag_name).cloned().unwrap_or_default(),
        Err(e) => {
            eprintln!("leftwm-ctx-apps: failed to load ctx config: {e}");
            return;
        }
    };
    if apps.is_empty() {
        return;
    }

    let Some(desktop_index) = find_desktop_index(&run_wmctrl(&["-d"]), &tag_name) else {
        return;
    };
    let running_classes: Vec<String> = parse_wmctrl_lx(&run_wmctrl(&["-lx"]))
        .into_iter()
        .filter(|(desktop, _)| *desktop == desktop_index)
        .map(|(_, class)| class)
        .collect();

    for app in &apps {
        if !is_app_running(app, &running_classes) {
            let _ = Command::new("sh")
                .arg("-c")
                .arg(app)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_name_extracts_first_word_lowercased() {
        assert_eq!(exe_name("Wezterm -e tmuxinator docker"), "wezterm");
        assert_eq!(exe_name("qutebrowser"), "qutebrowser");
        assert_eq!(exe_name(""), "");
    }

    #[test]
    fn is_app_running_matches_case_insensitive_substring() {
        let running = vec!["chromium.Chromium".to_string()];
        assert!(is_app_running("chromium --new-window", &running));
        assert!(!is_app_running("qutebrowser", &running));
    }

    #[test]
    fn is_app_running_false_for_empty_command() {
        let running = vec!["xterm.XTerm".to_string()];
        assert!(!is_app_running("", &running));
    }

    #[test]
    fn find_desktop_index_matches_exact_trailing_name() {
        let output = "0  - DG: N/A  VP: 0,0        WA: N/A  web\n\
                       1  * DG: N/A  VP: 0,0        WA: N/A  llm\n\
                       3  - DG: N/A  VP: 0,0        WA: N/A  docker-docker\n";
        assert_eq!(find_desktop_index(output, "docker-docker"), Some(3));
        assert_eq!(find_desktop_index(output, "nonexistent"), None);
    }

    #[test]
    fn parse_wmctrl_lx_extracts_desktop_and_class() {
        let output = "0x0070001d  0 chromium.Chromium     desktop canopy - Chromium\n\
                       0x01000022 12 qutebrowser.qutebrowser  desktop Superpowers\n";
        let parsed = parse_wmctrl_lx(output);
        assert_eq!(
            parsed,
            vec![
                (0, "chromium.Chromium".to_string()),
                (12, "qutebrowser.qutebrowser".to_string()),
            ]
        );
    }
}
