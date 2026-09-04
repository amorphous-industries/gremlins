use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use log::warn;
use serde_json::Value;

/// Default name of the project-local overlay directory.
pub const OVERLAY_DIRNAME: &str = ".gremlins";

// ---------------------------------------------------------------------------
// Path overrides from config.json "paths" section
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct PathOverrides {
    pub state_root: Option<PathBuf>,
    pub work_root: Option<PathBuf>,
    pub config_root: Option<PathBuf>,
    pub project_root: Option<PathBuf>,
    pub overlay_dir: Option<PathBuf>,
    pub scratch_root: Option<PathBuf>,
}

fn parse_path_overrides(paths: &HashMap<String, Value>) -> PathOverrides {
    fn str_to_path(v: &Value) -> Option<PathBuf> {
        v.as_str().map(PathBuf::from)
    }
    PathOverrides {
        state_root: paths.get("state-root").and_then(str_to_path),
        work_root: paths.get("work-root").and_then(str_to_path),
        config_root: paths.get("config-root").and_then(str_to_path),
        project_root: paths.get("project-root").and_then(str_to_path),
        overlay_dir: paths.get("overlay-dir").and_then(str_to_path),
        scratch_root: paths.get("scratch-root").and_then(str_to_path),
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Parsed content of config.json.
#[derive(Debug, Clone, Default)]
pub struct Config {
    raw: HashMap<String, Value>,
    default_client: Option<String>,
    exact_stage_clients: HashMap<String, String>,
    prefix_stage_clients: HashMap<String, String>,
    path_overrides: PathOverrides,
}

impl Config {
    /// Load from `user_config_root(None) / "config.json"`.
    /// Returns `Config::default()` if the file doesn't exist.
    pub fn load() -> Result<Self, ConfigError> {
        let path = resolve_user_config_root(None).join("config.json");
        let raw = match parse_json_config(&path) {
            Ok(raw) => raw,
            Err(ConfigError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(e) => return Err(e),
        };

        let default_client = raw
            .get("default-client")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        let (exact_stage_clients, prefix_stage_clients) = parse_stage_clients(&raw);

        let path_overrides = raw
            .get("paths")
            .and_then(|v| v.as_object())
            .map(|obj| {
                let m: HashMap<String, Value> =
                    obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                parse_path_overrides(&m)
            })
            .unwrap_or_default();

        Ok(Config {
            raw,
            default_client,
            exact_stage_clients,
            prefix_stage_clients,
            path_overrides,
        })
    }

    pub fn default_client(&self) -> Option<&str> {
        self.default_client.as_deref()
    }

    /// Returns `(exact_map, prefix_map)` from `default-client-by-stage`.
    pub fn default_client_by_stage(&self) -> (&HashMap<String, String>, &HashMap<String, String>) {
        (&self.exact_stage_clients, &self.prefix_stage_clients)
    }

    pub fn raw(&self) -> &HashMap<String, Value> {
        &self.raw
    }

    pub fn path_overrides(&self) -> &PathOverrides {
        &self.path_overrides
    }

    pub fn overlay_dirname(&self) -> &'static str {
        OVERLAY_DIRNAME
    }
}

// ---------------------------------------------------------------------------
// Stage client parsing
// ---------------------------------------------------------------------------

fn parse_stage_clients(
    raw: &HashMap<String, Value>,
) -> (HashMap<String, String>, HashMap<String, String>) {
    let obj = match raw
        .get("default-client-by-stage")
        .and_then(|v| v.as_object())
    {
        Some(o) => o,
        None => return (HashMap::new(), HashMap::new()),
    };

    let mut exact = HashMap::new();
    let mut prefix = HashMap::new();

    for (key, value) in obj {
        let val_str = match value.as_str() {
            Some(s) => s,
            None => {
                warn!(
                    "config key {:?} in default-client-by-stage has non-string value {:?} — skipping",
                    key, value
                );
                continue;
            }
        };

        if let Some(p) = key.strip_suffix('*') {
            if p.is_empty() {
                warn!(
                    "config key {:?} in default-client-by-stage produces an empty prefix, \
                     which would match every stage — skipping",
                    key
                );
                continue;
            }
            prefix.insert(p.to_string(), val_str.to_string());
        } else {
            exact.insert(key.clone(), val_str.to_string());
        }
    }

    (exact, prefix)
}

// ---------------------------------------------------------------------------
// JSON parsing
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("config file must contain a JSON object")]
    NotAnObject,
}

fn parse_json_config(path: &Path) -> Result<HashMap<String, Value>, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&content)?;
    match value {
        Value::Object(map) => Ok(map.into_iter().collect()),
        _ => Err(ConfigError::NotAnObject),
    }
}

// ---------------------------------------------------------------------------
// Process-global singleton
// ---------------------------------------------------------------------------

static GLOBAL_CONFIG: Mutex<Option<Arc<Config>>> = Mutex::new(None);

pub fn init_global() -> Result<(), ConfigError> {
    *GLOBAL_CONFIG.lock().unwrap() = Some(Arc::new(Config::load()?));
    Ok(())
}

pub fn get_global() -> Option<Arc<Config>> {
    GLOBAL_CONFIG.lock().unwrap().clone()
}

/// Get the global config, loading lazily on first access.
pub fn global_config() -> Result<Arc<Config>, ConfigError> {
    let mut guard = GLOBAL_CONFIG.lock().unwrap();
    if let Some(ref cfg) = *guard {
        return Ok(cfg.clone());
    }
    let cfg = Arc::new(Config::load()?);
    *guard = Some(cfg.clone());
    Ok(cfg)
}

pub fn clear_global() {
    *GLOBAL_CONFIG.lock().unwrap() = None;
}

// ---------------------------------------------------------------------------
// Env-var helpers
// ---------------------------------------------------------------------------

fn sandbox_override(subdir: &str) -> Option<PathBuf> {
    std::env::var("GREMLINS_SANDBOX_ROOT")
        .ok()
        .map(|root| PathBuf::from(root).join(subdir))
}

fn project_root_env_override() -> Option<PathBuf> {
    std::env::var("GREMLINS_PROJECT_ROOT")
        .ok()
        .map(PathBuf::from)
}

fn overlay_dir_env_override() -> Option<PathBuf> {
    std::env::var("GREMLINS_OVERLAY_DIR")
        .ok()
        .map(PathBuf::from)
}

// ---------------------------------------------------------------------------
// Internal path resolvers — pure, no global dependency
// ---------------------------------------------------------------------------

pub fn resolve_user_config_root(overrides: Option<&PathOverrides>) -> PathBuf {
    if let Some(sandbox) = sandbox_override("config") {
        return sandbox;
    }
    if let Some(o) = overrides {
        if let Some(ref p) = o.config_root {
            return p.clone();
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("gremlins")
}

pub fn resolve_state_root(overrides: Option<&PathOverrides>) -> PathBuf {
    if let Some(sandbox) = sandbox_override("state") {
        let p = sandbox;
        std::fs::create_dir_all(&p).ok();
        return p;
    }
    if let Some(o) = overrides {
        if let Some(ref p) = o.state_root {
            std::fs::create_dir_all(p).ok();
            return p.clone();
        }
    }
    let p = dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gremlins");
    std::fs::create_dir_all(&p).ok();
    p
}

pub fn resolve_work_root(overrides: Option<&PathOverrides>) -> PathBuf {
    if let Some(sandbox) = sandbox_override("work") {
        let p = sandbox;
        std::fs::create_dir_all(&p).ok();
        return p;
    }
    if let Some(o) = overrides {
        if let Some(ref p) = o.work_root {
            std::fs::create_dir_all(p).ok();
            return p.clone();
        }
    }
    let p = std::env::temp_dir().join("gremlins");
    std::fs::create_dir_all(&p).ok();
    p
}

pub fn resolve_project_root(overrides: Option<&PathOverrides>) -> PathBuf {
    if let Some(p) = project_root_env_override() {
        return p;
    }
    if let Some(o) = overrides {
        if let Some(ref p) = o.project_root {
            return p.clone();
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub fn resolve_project_overlay_dir(
    overrides: Option<&PathOverrides>,
    project_root: &Path,
) -> PathBuf {
    if let Some(p) = overlay_dir_env_override() {
        return p;
    }
    if let Some(o) = overrides {
        if let Some(ref p) = o.overlay_dir {
            return p.clone();
        }
    }
    project_root.join(OVERLAY_DIRNAME)
}

pub fn resolve_scratch_root(
    overrides: Option<&PathOverrides>,
    gremlin_id: Option<&str>,
) -> PathBuf {
    let sub = gremlin_id.unwrap_or("direct");
    if let Some(sandbox) = sandbox_override("scratch") {
        let p = sandbox.join(sub);
        std::fs::create_dir_all(&p).ok();
        return p;
    }
    if let Some(o) = overrides {
        if let Some(ref base) = o.scratch_root {
            let p = base.join(sub);
            std::fs::create_dir_all(&p).ok();
            return p;
        }
    }
    let p = std::env::temp_dir().join("gremlins-scratch").join(sub);
    std::fs::create_dir_all(&p).ok();
    p
}

// ---------------------------------------------------------------------------
// Public entry points — use the process-global config's overrides
// ---------------------------------------------------------------------------

pub fn state_root() -> PathBuf {
    let overrides = get_global().map(|c| c.path_overrides().clone());
    resolve_state_root(overrides.as_ref())
}

pub fn work_root() -> PathBuf {
    let overrides = get_global().map(|c| c.path_overrides().clone());
    resolve_work_root(overrides.as_ref())
}

pub fn user_config_root() -> PathBuf {
    // Bootstrap: never consult config.json for config-root during load.
    // Post-bootstrap, honour the override.
    let overrides = get_global().map(|c| c.path_overrides().clone());
    resolve_user_config_root(overrides.as_ref())
}

pub fn project_root() -> PathBuf {
    let overrides = get_global().map(|c| c.path_overrides().clone());
    resolve_project_root(overrides.as_ref())
}

pub fn overlay_dirname() -> &'static str {
    OVERLAY_DIRNAME
}

pub fn project_overlay_dir(project_root: &Path) -> PathBuf {
    let overrides = get_global().map(|c| c.path_overrides().clone());
    resolve_project_overlay_dir(overrides.as_ref(), project_root)
}

pub fn scratch_root(gremlin_id: Option<&str>) -> PathBuf {
    let overrides = get_global().map(|c| c.path_overrides().clone());
    resolve_scratch_root(overrides.as_ref(), gremlin_id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex as StdMutex;

    // Serialize env-var tests to prevent races.
    static ENV_MUTEX: StdMutex<()> = StdMutex::new(());

    fn clear_sandbox_env() {
        std::env::remove_var("GREMLINS_SANDBOX_ROOT");
        std::env::remove_var("GREMLINS_PROJECT_ROOT");
        std::env::remove_var("GREMLINS_OVERLAY_DIR");
    }

    // -----------------------------------------------------------------------
    // Config parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_config_default_client() {
        let raw: HashMap<String, Value> =
            serde_json::from_str(r#"{"default-client": "openai:gpt-4o"}"#).unwrap();
        let cfg = Config {
            raw: raw.clone(),
            default_client: raw
                .get("default-client")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from),
            exact_stage_clients: HashMap::new(),
            prefix_stage_clients: HashMap::new(),
            path_overrides: PathOverrides::default(),
        };
        assert_eq!(cfg.default_client(), Some("openai:gpt-4o"));
    }

    #[test]
    fn test_config_default_client_empty_string() {
        let raw: HashMap<String, Value> =
            serde_json::from_str(r#"{"default-client": ""}"#).unwrap();
        let cfg = Config {
            raw: raw.clone(),
            default_client: raw
                .get("default-client")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from),
            exact_stage_clients: HashMap::new(),
            prefix_stage_clients: HashMap::new(),
            path_overrides: PathOverrides::default(),
        };
        assert_eq!(cfg.default_client(), None);
    }

    #[test]
    fn test_config_default_client_by_stage() {
        let raw: HashMap<String, Value> = serde_json::from_str(
            r#"{"default-client-by-stage": {"local-review-*": "openrouter:doomclientv5", "plan-*": "openai:gpt-5"}}"#,
        )
        .unwrap();
        let (exact, prefix) = parse_stage_clients(&raw);
        assert!(exact.is_empty());
        assert_eq!(prefix.len(), 2);
        assert_eq!(
            prefix.get("local-review-").unwrap(),
            "openrouter:doomclientv5"
        );
        assert_eq!(prefix.get("plan-").unwrap(), "openai:gpt-5");
    }

    #[test]
    fn test_config_exact_and_prefix() {
        let raw: HashMap<String, Value> = serde_json::from_str(
            r#"{"default-client-by-stage": {"review": "openai:gpt-5", "plan-*": "openai:gpt-4o"}}"#,
        )
        .unwrap();
        let (exact, prefix) = parse_stage_clients(&raw);
        assert_eq!(exact.get("review").unwrap(), "openai:gpt-5");
        assert_eq!(prefix.get("plan-").unwrap(), "openai:gpt-4o");
    }

    #[test]
    fn test_config_non_string_value_skipped() {
        let raw: HashMap<String, Value> = serde_json::from_str(
            r#"{"default-client-by-stage": {"prefix-*": 42, "valid-*": "openrouter:model"}}"#,
        )
        .unwrap();
        let (exact, prefix) = parse_stage_clients(&raw);
        assert!(exact.is_empty());
        assert_eq!(prefix.len(), 1);
        assert_eq!(prefix.get("valid-").unwrap(), "openrouter:model");
    }

    #[test]
    fn test_config_empty_prefix_star_skipped() {
        let raw: HashMap<String, Value> = serde_json::from_str(
            r#"{"default-client-by-stage": {"*": "openrouter:model", "plan-*": "openai:gpt-5"}}"#,
        )
        .unwrap();
        let (exact, prefix) = parse_stage_clients(&raw);
        assert!(exact.is_empty());
        assert_eq!(prefix.len(), 1);
        assert_eq!(prefix.get("plan-").unwrap(), "openai:gpt-5");
    }

    #[test]
    fn test_config_json_decode_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        fs::write(&path, "{bad").unwrap();
        let result = parse_json_config(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_not_an_object() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("array.json");
        fs::write(&path, "[1, 2, 3]").unwrap();
        let result = parse_json_config(&path);
        assert!(matches!(result, Err(ConfigError::NotAnObject)));
    }

    #[test]
    fn test_config_file_not_found() {
        let result = parse_json_config(Path::new("/nonexistent/config.json"));
        assert!(matches!(result, Err(ConfigError::Io(_))));
    }

    #[test]
    fn test_paths_section_absent() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let dir = tempfile::tempdir().unwrap();
        let config_json = dir.path().join("config.json");
        fs::write(&config_json, r#"{"default-client": "a:b"}"#).unwrap();
        std::env::set_var("GREMLINS_SANDBOX_ROOT", dir.path());
        let cfg = Config::load().unwrap();
        let overrides = cfg.path_overrides();
        assert!(overrides.state_root.is_none());
        assert!(overrides.work_root.is_none());
        clear_sandbox_env();
    }

    // -----------------------------------------------------------------------
    // Path resolution tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_state_root_env_override() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("GREMLINS_SANDBOX_ROOT", dir.path());
        let result = resolve_state_root(None);
        assert_eq!(result, dir.path().join("state"));
        assert!(result.exists());
        clear_sandbox_env();
    }

    #[test]
    fn test_state_root_config_override() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let dir = tempfile::tempdir().unwrap();
        let overrides = PathOverrides {
            state_root: Some(dir.path().join("my-state")),
            ..Default::default()
        };
        let result = resolve_state_root(Some(&overrides));
        assert_eq!(result, dir.path().join("my-state"));
        assert!(result.exists());
        clear_sandbox_env();
    }

    #[test]
    fn test_state_root_default() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let result = resolve_state_root(None);
        // Should be under the platform state dir
        assert!(result.to_str().unwrap().contains("gremlins"));
        assert!(result.exists());
    }

    #[test]
    fn test_project_root_env_override() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("GREMLINS_PROJECT_ROOT", dir.path());
        let result = resolve_project_root(None);
        assert_eq!(result, dir.path());
        clear_sandbox_env();
    }

    #[test]
    fn test_project_root_config_override() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let dir = tempfile::tempdir().unwrap();
        let overrides = PathOverrides {
            project_root: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let result = resolve_project_root(Some(&overrides));
        assert_eq!(result, dir.path());
        clear_sandbox_env();
    }

    #[test]
    fn test_project_root_default() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let result = resolve_project_root(None);
        assert_eq!(result, std::env::current_dir().unwrap());
    }

    #[test]
    fn test_work_root_sandbox() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("GREMLINS_SANDBOX_ROOT", dir.path());
        let result = resolve_work_root(None);
        assert_eq!(result, dir.path().join("work"));
        assert!(result.exists());
        clear_sandbox_env();
    }

    #[test]
    fn test_work_root_default() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let result = resolve_work_root(None);
        assert!(result.to_str().unwrap().contains("gremlins"));
        assert!(result.exists());
    }

    #[test]
    fn test_user_config_root_sandbox() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("GREMLINS_SANDBOX_ROOT", dir.path());
        let result = resolve_user_config_root(None);
        assert_eq!(result, dir.path().join("config"));
        clear_sandbox_env();
    }

    #[test]
    fn test_user_config_root_default() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let result = resolve_user_config_root(None);
        assert!(result.to_str().unwrap().contains("gremlins"));
    }

    #[test]
    fn test_project_overlay_dir_env() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("GREMLINS_OVERLAY_DIR", dir.path());
        let result = resolve_project_overlay_dir(None, Path::new("/fake/project"));
        assert_eq!(result, dir.path());
        clear_sandbox_env();
    }

    #[test]
    fn test_project_overlay_dir_config() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let dir = tempfile::tempdir().unwrap();
        let overrides = PathOverrides {
            overlay_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let result = resolve_project_overlay_dir(Some(&overrides), Path::new("/fake/project"));
        assert_eq!(result, dir.path());
        clear_sandbox_env();
    }

    #[test]
    fn test_project_overlay_dir_default() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let result = resolve_project_overlay_dir(None, Path::new("/fake/project"));
        assert_eq!(result, Path::new("/fake/project").join(".gremlins"));
    }

    #[test]
    fn test_scratch_root_sandbox() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("GREMLINS_SANDBOX_ROOT", dir.path());
        let result = resolve_scratch_root(None, Some("my-gremlin"));
        assert_eq!(result, dir.path().join("scratch").join("my-gremlin"));
        assert!(result.exists());
        clear_sandbox_env();
    }

    #[test]
    fn test_scratch_root_default() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let result = resolve_scratch_root(None, Some("my-gremlin"));
        assert!(result.to_str().unwrap().contains("gremlins-scratch"));
        assert!(result.to_str().unwrap().contains("my-gremlin"));
        assert!(result.exists());
    }

    #[test]
    fn test_scratch_root_no_id() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let result = resolve_scratch_root(None, None);
        assert!(result.to_str().unwrap().contains("direct"));
        assert!(result.exists());
    }

    #[test]
    fn test_precedence_env_over_config() {
        let _guard = ENV_MUTEX.lock().unwrap();
        clear_sandbox_env();
        let env_dir = tempfile::tempdir().unwrap();
        let cfg_dir = tempfile::tempdir().unwrap();
        std::env::set_var("GREMLINS_SANDBOX_ROOT", env_dir.path());

        let overrides = PathOverrides {
            state_root: Some(cfg_dir.path().join("cfg-state")),
            ..Default::default()
        };
        let result = resolve_state_root(Some(&overrides));
        // Env var wins
        assert_eq!(result, env_dir.path().join("state"));
        clear_sandbox_env();
    }

    #[test]
    fn test_global_init_clear() {
        clear_global();
        assert!(get_global().is_none());
        init_global().unwrap();
        assert!(get_global().is_some());
        clear_global();
        assert!(get_global().is_none());
    }

    #[test]
    fn test_global_lazy_load() {
        clear_global();
        let cfg1 = global_config().unwrap();
        let cfg2 = global_config().unwrap();
        // Same Arc — lazy load only happens once
        assert!(Arc::ptr_eq(&cfg1, &cfg2));
    }

    #[test]
    fn test_global_after_clear_reloads() {
        clear_global();
        let cfg1 = global_config().unwrap();
        clear_global();
        let cfg2 = global_config().unwrap();
        // After clear, a new Arc is created
        assert!(!Arc::ptr_eq(&cfg1, &cfg2));
    }
}
