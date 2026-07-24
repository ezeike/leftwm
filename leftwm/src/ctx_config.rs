use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Mirrors `XMonad.Config.Workspaces.defaultWorkspaceTypes`
/// (~/config/wm/xmonad/Workspaces.hs:167-168).
const DEFAULT_SLOT_NAMES: [&str; 3] = ["web", "llm", "emacs"];

#[derive(Debug, Deserialize)]
struct CtxFile {
    workspaces: Vec<CtxWorkspace>,
}

#[derive(Debug, Deserialize)]
struct CtxWorkspace {
    name: String,
    #[serde(default)]
    wm_mode: WsMode,
    #[serde(default)]
    wm_workspaces: HashMap<u8, CtxSlot>,
}

#[derive(Debug, Deserialize, Default)]
struct CtxSlot {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    apps: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Default)]
enum WsMode {
    #[default]
    Disabled,
    Show,
    Hide,
}

/// Mirrors `XMonad.Config.Workspaces`'s `FromJSON WsMode` instance
/// (~/config/wm/xmonad/Workspaces.hs:55-63): string "show"/"hide"
/// (case-insensitive), legacy bool true/false, or absent/null all parse;
/// any other string is a hard parse error (matching the Haskell `fail`
/// there, which aborts the whole document, not just this one field).
impl<'de> Deserialize<'de> for WsMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Bool(bool),
            Str(String),
        }
        let raw = Option::<Raw>::deserialize(deserializer)?;
        Ok(match raw {
            None => WsMode::Disabled,
            Some(Raw::Bool(true)) => WsMode::Show,
            Some(Raw::Bool(false)) => WsMode::Disabled,
            Some(Raw::Str(s)) => match s.to_lowercase().as_str() {
                "show" => WsMode::Show,
                "hide" => WsMode::Hide,
                other => {
                    return Err(serde::de::Error::custom(format!(
                        "invalid wm_mode value: {other:?}. Expected 'show' or 'hide'"
                    )));
                }
            },
        })
    }
}

/// Parses `ctx/config.yaml` and derives leftwm tag names, mirroring
/// `XMonad.Config.Workspaces.generateWorkspaceNames`
/// (~/config/wm/xmonad/Workspaces.hs:171-183) exactly: for each workspace
/// whose `wm_mode` is `show` or `hide` (absent/`Disabled` workspaces are
/// skipped entirely), emit `"<name>-<slot>"` for slots 1..=3, using that
/// slot's `wm_workspaces[i].name` override if present, else the default
/// `["web", "llm", "emacs"]` name for that position.
pub fn derive_tags(ctx_yaml_path: &Path) -> Result<Vec<String>, String> {
    let file = load_ctx_file(ctx_yaml_path)?;

    let mut tags = Vec::new();
    for ws in &file.workspaces {
        if ws.wm_mode == WsMode::Disabled {
            continue;
        }
        for i in 1u8..=3 {
            let slot_name = ws
                .wm_workspaces
                .get(&i)
                .and_then(|slot| slot.name.clone())
                .unwrap_or_else(|| DEFAULT_SLOT_NAMES[(i - 1) as usize].to_string());
            tags.push(format!("{}-{}", ws.name, slot_name));
        }
    }
    Ok(tags)
}

fn load_ctx_file(ctx_yaml_path: &Path) -> Result<CtxFile, String> {
    let contents = std::fs::read_to_string(ctx_yaml_path)
        .map_err(|e| format!("reading {}: {e}", ctx_yaml_path.display()))?;
    serde_yaml::from_str(&contents).map_err(|e| format!("parsing {}: {e}", ctx_yaml_path.display()))
}

/// Parses `ctx/config.yaml` and maps each derived tag name (same naming as
/// `derive_tags`) to its configured `apps:` list, mirroring xmonad's
/// `TopicItem`/`wstApps` app-spawn config
/// (~/config/wm/xmonad/Workspaces.hs). Tags whose slot has no `apps:` key
/// map to an empty `Vec`, not an absent map entry — every non-disabled
/// workspace's 3 slots always get an entry, matching `derive_tags`'
/// unconditional 3-tags-per-workspace emission.
pub fn tag_apps(ctx_yaml_path: &Path) -> Result<HashMap<String, Vec<String>>, String> {
    let file = load_ctx_file(ctx_yaml_path)?;

    let mut apps_by_tag = HashMap::new();
    for ws in &file.workspaces {
        if ws.wm_mode == WsMode::Disabled {
            continue;
        }
        for i in 1u8..=3 {
            let slot = ws.wm_workspaces.get(&i);
            let slot_name = slot
                .and_then(|s| s.name.clone())
                .unwrap_or_else(|| DEFAULT_SLOT_NAMES[(i - 1) as usize].to_string());
            let tag = format!("{}-{}", ws.name, slot_name);
            let apps = slot.map(|s| s.apps.clone()).unwrap_or_default();
            apps_by_tag.insert(tag, apps);
        }
    }
    Ok(apps_by_tag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_fixture(contents: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().expect("create temp fixture file");
        file.write_all(contents.as_bytes()).expect("write fixture");
        file
    }

    #[test]
    fn derives_default_slot_names_when_no_overrides() {
        let file = write_fixture(
            r#"
workspaces:
  - name: "foo"
    working_directory: "~/"
    wm_mode: "show"
"#,
        );
        let tags = derive_tags(file.path()).unwrap();
        assert_eq!(tags, vec!["foo-web", "foo-llm", "foo-emacs"]);
    }

    #[test]
    fn custom_slot_name_overrides_the_default() {
        let file = write_fixture(
            r#"
workspaces:
  - name: "docker"
    working_directory: "~/"
    wm_mode: "show"
    wm_workspaces:
      1:
        name: "docker"
        apps:
          - "wezterm -e tmuxinator docker"
"#,
        );
        let tags = derive_tags(file.path()).unwrap();
        assert_eq!(tags, vec!["docker-docker", "docker-llm", "docker-emacs"]);
    }

    #[test]
    fn wm_mode_hide_still_generates_tags() {
        let file = write_fixture(
            r#"
workspaces:
  - name: "oracle-logs"
    working_directory: "~/"
    wm_mode: "hide"
"#,
        );
        let tags = derive_tags(file.path()).unwrap();
        assert_eq!(tags, vec!["oracle-logs-web", "oracle-logs-llm", "oracle-logs-emacs"]);
    }

    #[test]
    fn missing_wm_mode_excludes_the_workspace() {
        let file = write_fixture(
            r#"
workspaces:
  - name: "scratchpad"
    working_directory: "~/"
"#,
        );
        let tags = derive_tags(file.path()).unwrap();
        assert_eq!(tags, Vec::<String>::new());
    }

    #[test]
    fn invalid_wm_mode_string_is_an_error() {
        let file = write_fixture(
            r#"
workspaces:
  - name: "foo"
    working_directory: "~/"
    wm_mode: "sideways"
"#,
        );
        assert!(derive_tags(file.path()).is_err());
    }

    #[test]
    fn legacy_bool_true_is_same_as_show() {
        let file = write_fixture(
            r#"
workspaces:
  - name: "foo"
    working_directory: "~/"
    wm_mode: true
"#,
        );
        let tags = derive_tags(file.path()).unwrap();
        assert_eq!(tags, vec!["foo-web", "foo-llm", "foo-emacs"]);
    }

    #[test]
    fn legacy_bool_false_is_same_as_disabled() {
        let file = write_fixture(
            r#"
workspaces:
  - name: "foo"
    working_directory: "~/"
    wm_mode: false
"#,
        );
        let tags = derive_tags(file.path()).unwrap();
        assert_eq!(tags, Vec::<String>::new());
    }

    #[test]
    fn wm_mode_string_matching_is_case_insensitive() {
        let file = write_fixture(
            r#"
workspaces:
  - name: "foo"
    working_directory: "~/"
    wm_mode: "SHOW"
  - name: "bar"
    working_directory: "~/"
    wm_mode: "Hide"
"#,
        );
        let tags = derive_tags(file.path()).unwrap();
        assert_eq!(
            tags,
            vec![
                "foo-web", "foo-llm", "foo-emacs",
                "bar-web", "bar-llm", "bar-emacs",
            ]
        );
    }

    #[test]
    fn explicit_null_is_same_as_absent() {
        let file = write_fixture(
            r#"
workspaces:
  - name: "foo"
    working_directory: "~/"
    wm_mode: ~
  - name: "bar"
    working_directory: "~/"
    wm_mode: null
"#,
        );
        let tags = derive_tags(file.path()).unwrap();
        assert_eq!(tags, Vec::<String>::new());
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(derive_tags(std::path::Path::new("/nonexistent/ctx-config.yaml")).is_err());
    }

    #[test]
    fn tag_apps_returns_configured_apps_per_tag() {
        let file = write_fixture(
            r#"
workspaces:
  - name: "docker"
    working_directory: "~/"
    wm_mode: "show"
    wm_workspaces:
      1:
        name: "docker"
        apps:
          - "wezterm -e tmuxinator docker"
      2:
        apps:
          - "qutebrowser"
"#,
        );
        let apps = tag_apps(file.path()).unwrap();
        assert_eq!(
            apps.get("docker-docker"),
            Some(&vec!["wezterm -e tmuxinator docker".to_string()])
        );
        assert_eq!(apps.get("docker-llm"), Some(&vec!["qutebrowser".to_string()]));
    }

    #[test]
    fn tag_apps_defaults_to_empty_list_when_slot_has_no_apps() {
        let file = write_fixture(
            r#"
workspaces:
  - name: "foo"
    working_directory: "~/"
    wm_mode: "show"
"#,
        );
        let apps = tag_apps(file.path()).unwrap();
        assert_eq!(apps.get("foo-web"), Some(&Vec::<String>::new()));
        assert_eq!(apps.get("foo-llm"), Some(&Vec::<String>::new()));
        assert_eq!(apps.get("foo-emacs"), Some(&Vec::<String>::new()));
    }

    #[test]
    fn tag_apps_excludes_disabled_workspaces() {
        let file = write_fixture(
            r#"
workspaces:
  - name: "scratchpad"
    working_directory: "~/"
"#,
        );
        let apps = tag_apps(file.path()).unwrap();
        assert!(apps.is_empty());
    }

    #[test]
    fn tag_apps_missing_file_is_an_error() {
        assert!(tag_apps(std::path::Path::new("/nonexistent/ctx-config.yaml")).is_err());
    }

    #[test]
    fn multiple_workspaces_concatenate_in_order() {
        let file = write_fixture(
            r#"
workspaces:
  - name: "foo"
    working_directory: "~/"
    wm_mode: "show"
  - name: "bar"
    working_directory: "~/"
    wm_mode: "show"
"#,
        );
        let tags = derive_tags(file.path()).unwrap();
        assert_eq!(
            tags,
            vec![
                "foo-web", "foo-llm", "foo-emacs",
                "bar-web", "bar-llm", "bar-emacs",
            ]
        );
    }
}
