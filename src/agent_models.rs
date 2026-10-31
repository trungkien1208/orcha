//! Model catalogs and observed model settings, read from the same places each
//! agent CLI uses: Claude Code's model-catalog cache, Codex's models cache,
//! and `cursor-agent models`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::api::schema::{AgentModelInfo, AgentModelVariant};

/// `cursor-agent models` asks Cursor's servers; reuse its answer for a while.
const CURSOR_CATALOG_TTL: Duration = Duration::from_secs(600);
const CURSOR_MODELS_TIMEOUT: Duration = Duration::from_secs(20);

/// Efforts in the order agents list them, used to split Cursor variant ids.
const CURSOR_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

pub(crate) struct ModelCatalog {
    pub source: String,
    pub models: Vec<AgentModelInfo>,
}

pub(crate) fn catalog(kind: &str) -> Result<ModelCatalog, String> {
    match kind {
        "claude" => claude_catalog(&claude_config_dir()),
        "codex" => codex_catalog(&codex_home()),
        "cursor" => cursor_catalog(),
        other => Err(format!(
            "no model catalog for {other}; supported kinds are claude, codex and cursor"
        )),
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn claude_config_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".claude"))
}

fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".codex"))
}

fn claude_catalog(config_dir: &Path) -> Result<ModelCatalog, String> {
    let dir = config_dir.join("cache").join("model-catalog");
    let newest = std::fs::read_dir(&dir)
        .map_err(|err| {
            format!(
                "Claude Code model catalog not found at {}: {err}",
                dir.display()
            )
        })?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with("-cc.json"))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
        .ok_or_else(|| {
            format!(
                "Claude Code model catalog not found in {}; start Claude Code once",
                dir.display()
            )
        })?;
    let text = std::fs::read_to_string(&newest).map_err(|err| err.to_string())?;
    Ok(ModelCatalog {
        source: newest.display().to_string(),
        models: parse_claude_catalog(&text)?,
    })
}

pub(crate) fn parse_claude_catalog(text: &str) -> Result<Vec<AgentModelInfo>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|err| format!("invalid Claude catalog: {err}"))?;
    let models = value
        .pointer("/catalog/config/models")
        .and_then(serde_json::Value::as_array)
        .ok_or("Claude catalog has no models")?;
    Ok(models
        .iter()
        .filter_map(|model| {
            let id = model.get("id")?.as_str()?.to_string();
            let runtime = model.get("runtime");
            let efforts = runtime
                .and_then(|runtime| runtime.get("effort_levels"))
                .and_then(serde_json::Value::as_array)
                .map(|levels| {
                    levels
                        .iter()
                        .filter_map(|level| level.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            Some(AgentModelInfo {
                name: text_field(model, "name").unwrap_or_else(|| id.clone()),
                id,
                efforts,
                default_effort: runtime.and_then(|runtime| text_field(runtime, "default_effort")),
                fast_available: model.get("fast_mode").is_some_and(|fast| !fast.is_null()),
                fast_efforts: Vec::new(),
                variants: Vec::new(),
                default: false,
            })
        })
        .collect())
}

fn codex_catalog(codex_home: &Path) -> Result<ModelCatalog, String> {
    let path = codex_home.join("models_cache.json");
    let text = std::fs::read_to_string(&path).map_err(|err| {
        format!(
            "Codex models cache not found at {}: {err}; start Codex once",
            path.display()
        )
    })?;
    Ok(ModelCatalog {
        source: path.display().to_string(),
        models: parse_codex_catalog(&text)?,
    })
}

pub(crate) fn parse_codex_catalog(text: &str) -> Result<Vec<AgentModelInfo>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|err| format!("invalid Codex models cache: {err}"))?;
    let models = value
        .get("models")
        .and_then(serde_json::Value::as_array)
        .ok_or("Codex models cache has no models")?;
    Ok(models
        .iter()
        // Codex's own picker shows only `list` models.
        .filter(|model| model.get("visibility").and_then(serde_json::Value::as_str) == Some("list"))
        .filter_map(|model| {
            let id = model.get("slug")?.as_str()?.to_string();
            let efforts = model
                .get("supported_reasoning_levels")
                .and_then(serde_json::Value::as_array)
                .map(|levels| {
                    levels
                        .iter()
                        .filter_map(|level| text_field(level, "effort"))
                        .collect()
                })
                .unwrap_or_default();
            let fast_available = model
                .get("additional_speed_tiers")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|tiers| tiers.iter().any(|tier| tier.as_str() == Some("fast")));
            Some(AgentModelInfo {
                name: text_field(model, "display_name").unwrap_or_else(|| id.clone()),
                id,
                efforts,
                default_effort: text_field(model, "default_reasoning_level"),
                fast_available,
                fast_efforts: Vec::new(),
                variants: Vec::new(),
                default: false,
            })
        })
        .collect())
}

fn cursor_catalog() -> Result<ModelCatalog, String> {
    static CACHE: Mutex<Option<(Instant, Vec<AgentModelInfo>)>> = Mutex::new(None);
    if let Ok(cache) = CACHE.lock() {
        if let Some((at, models)) = cache.as_ref() {
            if at.elapsed() < CURSOR_CATALOG_TTL {
                return Ok(ModelCatalog {
                    source: "cursor-agent models".into(),
                    models: models.clone(),
                });
            }
        }
    }
    let output = run_with_timeout("cursor-agent", &["models"], CURSOR_MODELS_TIMEOUT)?;
    let models = parse_cursor_models(&output);
    if models.is_empty() {
        return Err("cursor-agent models listed no models; sign in to Cursor first".into());
    }
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((Instant::now(), models.clone()));
    }
    Ok(ModelCatalog {
        source: "cursor-agent models".into(),
        models,
    })
}

fn run_with_timeout(program: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut child = crate::noninteractive_process::command(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|err| format!("could not run {program}: {err}"))?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut text);
        text
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program} {} timed out", args.join(" ")));
            }
            Err(err) => return Err(err.to_string()),
        }
    }
    reader
        .join()
        .map_err(|_| format!("could not read {program} output"))
}

/// Groups `cursor-agent models` lines (`id - Name`) into models whose
/// variants keep Cursor's own ids for each effort and fast combination.
pub(crate) fn parse_cursor_models(output: &str) -> Vec<AgentModelInfo> {
    let mut models: Vec<AgentModelInfo> = Vec::new();
    for line in strip_ansi(output).lines() {
        let line = line.trim().trim_matches('\u{200b}');
        let Some((id, name)) = line.split_once(" - ") else {
            continue;
        };
        let id = id.trim();
        if id.is_empty() || id.contains(' ') {
            continue;
        }
        let default = name.contains("(default)");
        let name = name.replace("(default)", "").replace('\u{200b}', "");
        let (base, effort, fast) = split_cursor_variant(id);
        let base_name = cursor_base_name(&name, effort.as_deref(), fast);
        let model = match models.iter_mut().find(|model| model.id == base) {
            Some(model) => model,
            None => {
                models.push(AgentModelInfo {
                    id: base.clone(),
                    name: base_name,
                    efforts: Vec::new(),
                    default_effort: None,
                    fast_available: false,
                    fast_efforts: Vec::new(),
                    variants: Vec::new(),
                    default: false,
                });
                models.last_mut().expect("just pushed")
            }
        };
        model.default |= default;
        if let Some(effort) = effort.as_ref() {
            if !model.efforts.contains(effort) {
                model.efforts.push(effort.clone());
            }
            if fast && !model.fast_efforts.contains(effort) {
                model.fast_efforts.push(effort.clone());
            }
        }
        model.fast_available |= fast;
        model.variants.push(AgentModelVariant {
            id: id.to_string(),
            effort,
            fast,
        });
    }
    for model in &mut models {
        // Only list per-effort fast support when it differs between efforts.
        if model.fast_efforts.len() == model.efforts.len() {
            model.fast_efforts.clear();
        }
    }
    models
}

fn split_cursor_variant(id: &str) -> (String, Option<String>, bool) {
    let (rest, fast) = match id.strip_suffix("-fast") {
        Some(rest) => (rest, true),
        None => (id, false),
    };
    for effort in CURSOR_EFFORTS {
        if let Some(base) = rest.strip_suffix(&format!("-{effort}")) {
            return (base.to_string(), Some(effort.to_string()), fast);
        }
    }
    (rest.to_string(), None, fast)
}

fn cursor_base_name(name: &str, effort: Option<&str>, fast: bool) -> String {
    let mut words: Vec<&str> = name.split_whitespace().collect();
    if fast
        && words
            .last()
            .is_some_and(|word| word.eq_ignore_ascii_case("fast"))
    {
        words.pop();
    }
    let effort_word = match effort {
        Some("xhigh") => Some("Extra High"),
        Some("low") => Some("Low"),
        Some("medium") => Some("Medium"),
        Some("high") => Some("High"),
        Some("max") => Some("Max"),
        _ => None,
    };
    let mut joined = words.join(" ");
    if let Some(word) = effort_word {
        if let Some(stripped) = joined.strip_suffix(word) {
            joined = stripped.trim_end().to_string();
        }
    }
    joined
}

fn text_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_catalog_lists_efforts_default_and_fast() {
        let text = r#"{"catalog":{"config":{"models":[
            {"id":"claude-opus-5-5","name":"Opus 5.5","fast_mode":{"type":"toggle"},
             "runtime":{"effort_levels":["low","medium","high"],"default_effort":"medium"}},
            {"id":"claude-sonnet-5-5","name":"Sonnet 5.5","runtime":{"effort_levels":["low"]}}
        ]}}}"#;
        let models = parse_claude_catalog(text).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "claude-opus-5-5");
        assert_eq!(models[0].efforts, ["low", "medium", "high"]);
        assert_eq!(models[0].default_effort.as_deref(), Some("medium"));
        assert!(models[0].fast_available);
        assert!(!models[1].fast_available);
    }

    #[test]
    fn codex_catalog_keeps_only_listed_models() {
        let text = r#"{"models":[
            {"slug":"gpt-a","display_name":"GPT A","visibility":"list","default_reasoning_level":"low",
             "supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}],"additional_speed_tiers":["fast"]},
            {"slug":"hidden","visibility":"hide"}
        ]}"#;
        let models = parse_codex_catalog(text).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "GPT A");
        assert_eq!(models[0].efforts, ["low", "high"]);
        assert!(models[0].fast_available);
    }

    #[test]
    fn cursor_models_group_variants_by_effort_and_fast() {
        let output = "\u{1b}[1mAvailable models\u{1b}[0m\n\nauto - Auto (default)\n\
            grok-4.7-low - Grok 4.7  Low\ngrok-4.7-low-fast - Grok 4.7  Low Fast\u{200b}\u{200b}\n\
            grok-4.7-xhigh - Grok 4.7  Extra High\ncomposer-2.5 - Composer 2.5\n";
        let models = parse_cursor_models(output);
        let ids: Vec<_> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(ids, ["auto", "grok-4.7", "composer-2.5"]);
        assert!(models[0].default);
        let grok = &models[1];
        assert_eq!(grok.name, "Grok 4.7");
        assert_eq!(grok.efforts, ["low", "xhigh"]);
        assert!(grok.fast_available);
        assert_eq!(grok.fast_efforts, ["low"], "fast only exists for low");
        assert_eq!(grok.variants[1].id, "grok-4.7-low-fast");
        assert!(grok.variants[1].fast);
    }
}
