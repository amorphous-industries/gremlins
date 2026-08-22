use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use regex::Regex;
use rig_core::completion::ToolDefinition;
use tokio::process::Command;

const GREP_MAX_LINES: usize = 2000;
const BASH_TIMEOUT_SECS: u64 = 120;
const SKIP_DIRS: &[&str] = &["__pycache__", "node_modules", "target"];

#[derive(Clone)]
pub struct ToolContext {
    pub cwd: Option<PathBuf>,
    pub extra_env: Option<HashMap<String, String>>,
    pub bypass: bool,
    pub worktree_root: PathBuf,
    pub audit_log: Option<PathBuf>,
    pub allowed_tools: Option<Vec<String>>,
}

pub fn project_root() -> PathBuf {
    match std::env::var("GREMLINS_PROJECT_ROOT") {
        Ok(s) if !s.is_empty() => PathBuf::from(s),
        _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    }
}

pub fn worktree_root(cwd: Option<&Path>) -> PathBuf {
    cwd.map(Path::to_path_buf).unwrap_or_else(project_root)
}

pub fn audit_log_path(raw_path: &Path) -> PathBuf {
    let stem = raw_path.file_stem().unwrap_or_default();
    let name = format!("{}.audit.jsonl", stem.to_string_lossy());
    match raw_path.parent() {
        Some(parent) => parent.join(name),
        None => PathBuf::from(name),
    }
}

pub fn resolve(file_path: &str, cwd: Option<&Path>) -> PathBuf {
    let p = PathBuf::from(file_path);
    if !p.is_absolute() {
        if let Some(cwd) = cwd {
            return cwd.join(p);
        }
    }
    p
}

fn normalize_dots(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn normalize_path(p: &Path) -> Option<PathBuf> {
    if let Ok(c) = p.canonicalize() {
        return Some(c);
    }
    let mut existing = p.to_path_buf();
    let mut rest: Vec<OsString> = Vec::new();
    loop {
        if let Ok(c) = existing.canonicalize() {
            let mut out = c;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return Some(normalize_dots(&out));
        }
        match existing.file_name() {
            Some(name) => {
                rest.push(name.to_os_string());
                if !existing.pop() {
                    break;
                }
            }
            None => break,
        }
    }
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(p)
    };
    Some(normalize_dots(&abs))
}

pub fn within_worktree(p: &Path, root: &Path) -> bool {
    match (normalize_path(p), normalize_path(root)) {
        (Some(p), Some(root)) => p.starts_with(root),
        _ => false,
    }
}

fn expand_user(s: &str) -> String {
    if s == "~" || s.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            if s == "~" {
                return home;
            }
            return format!("{home}{}", &s[1..]);
        }
    }
    s.to_string()
}

pub fn enforce(bypass: bool, root: &Path, pth: &str, cwd: Option<&Path>) -> Option<String> {
    if bypass {
        return None;
    }
    let p = resolve(pth, cwd);
    if !within_worktree(&p, root) {
        return Some(format!("Error: path outside worktree: {pth}"));
    }
    None
}

pub fn bash_check(bypass: bool, root: &Path, cmd: &str, cwd: Option<&Path>) -> Option<String> {
    if bypass {
        return None;
    }
    let s = cmd.trim();
    if s.is_empty() {
        return None;
    }
    for raw_tok in s.split_whitespace() {
        let tok = raw_tok.trim_matches(|c| c == '\'' || c == '"');
        if tok.is_empty() {
            continue;
        }
        let looks_like_path = tok.starts_with('/')
            || tok.starts_with('~')
            || tok.starts_with("..")
            || tok.contains('/');
        if !looks_like_path {
            continue;
        }
        let expanded = if tok.starts_with('~') {
            expand_user(tok)
        } else {
            tok.to_string()
        };
        let p = resolve(&expanded, cwd);
        if !within_worktree(&p, root) {
            return Some(format!("Error: path outside worktree: {raw_tok}"));
        }
    }
    None
}

fn audit_key_arg(args_json: &str) -> String {
    let Ok(d) = serde_json::from_str::<serde_json::Value>(args_json) else {
        return String::new();
    };
    if let Some(obj) = d.as_object() {
        for k in ["file_path", "command", "pattern", "path"] {
            if let Some(v) = obj.get(k).and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    return v.to_string();
                }
            }
        }
    }
    String::new()
}

fn audit(log: Option<&Path>, tool: &str, key_arg: &str, status: &str, bypass: bool) {
    let Some(log) = log else {
        return;
    };
    let truncated: String = key_arg.chars().take(200).collect();
    let entry = serde_json::json!({
        "ts": super::stream::ts_internal(),
        "tool": tool,
        "key_arg": truncated,
        "status": status,
        "bypass": bypass,
    });
    let Ok(line) = serde_json::to_string(&entry) else {
        return;
    };
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .and_then(|mut f| {
            use std::io::Write;
            writeln!(f, "{line}")
        });
}

fn result_status(res: &str) -> &'static str {
    if res.starts_with("Error:") || res.starts_with("[exit") || res.starts_with("[timeout]") {
        "error"
    } else {
        "ok"
    }
}

fn check_tool(name: &str, ctx: &ToolContext, args: &serde_json::Value) -> Option<String> {
    match name {
        "Read" | "Edit" | "Write" => enforce(
            ctx.bypass,
            &ctx.worktree_root,
            args.get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("."),
            ctx.cwd.as_deref(),
        ),
        "Grep" => enforce(
            ctx.bypass,
            &ctx.worktree_root,
            args.get("path").and_then(|v| v.as_str()).unwrap_or("."),
            ctx.cwd.as_deref(),
        ),
        "Glob" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
            let base = resolve(path, ctx.cwd.as_deref());
            let full_path = base.join(pattern);
            let full_str = full_path.to_string_lossy();
            // Reject absolute patterns and parent traversal that escape the base.
            // enforce resolves the argument, so passing the joined absolute path
            // correctly guards against e.g. {"path":".","pattern":"../*"}.
            enforce(ctx.bypass, &ctx.worktree_root, &full_str, None)
        }
        "Bash" => bash_check(
            ctx.bypass,
            &ctx.worktree_root,
            args.get("command").and_then(|v| v.as_str()).unwrap_or(""),
            ctx.cwd.as_deref(),
        ),
        _ => None,
    }
}

async fn blocking_string(f: impl FnOnce() -> String + Send + 'static) -> String {
    match tokio::task::spawn_blocking(f).await {
        Ok(s) => s,
        Err(e) => format!("Error: {e}"),
    }
}

fn parse_args(args_json: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(args_json).map_err(|e| format!("Error: invalid arguments: {e}"))
}

fn req_str<'a>(args: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("Error: missing '{key}'"))
}

pub async fn read_invoke(ctx: &ToolContext, args_json: &str) -> String {
    let cwd = ctx.cwd.clone();
    let args_json = args_json.to_string();
    blocking_string(move || read_sync(cwd.as_deref(), &args_json)).await
}

fn read_sync(cwd: Option<&Path>, args_json: &str) -> String {
    let args = match parse_args(args_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let file_path = match req_str(&args, "file_path") {
        Ok(s) => s,
        Err(e) => return e,
    };
    let path = resolve(file_path, cwd);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => return format!("Error: {e}"),
    };
    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);
    let mut lines: Vec<&str> = content.split_inclusive('\n').collect();
    if offset > 0 {
        lines = lines.into_iter().skip(offset).collect();
    }
    if let Some(limit) = limit {
        lines.truncate(limit);
    }
    lines.concat()
}

pub async fn edit_invoke(ctx: &ToolContext, args_json: &str) -> String {
    let cwd = ctx.cwd.clone();
    let args_json = args_json.to_string();
    blocking_string(move || edit_sync(cwd.as_deref(), &args_json)).await
}

fn edit_sync(cwd: Option<&Path>, args_json: &str) -> String {
    let args = match parse_args(args_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let file_path = match req_str(&args, "file_path") {
        Ok(s) => s,
        Err(e) => return e,
    };
    let old = match req_str(&args, "old_string") {
        Ok(s) => s,
        Err(e) => return e,
    };
    let new = match req_str(&args, "new_string") {
        Ok(s) => s,
        Err(e) => return e,
    };
    let path = resolve(file_path, cwd);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => return format!("Error: {e}"),
    };
    if old.is_empty() || !content.contains(old) {
        return format!("Error: old_string not found in {file_path}");
    }
    if content.matches(old).count() > 1 {
        return format!("Error: old_string is not unique in {file_path}");
    }
    let updated = content.replacen(old, new, 1);
    if let Err(e) = std::fs::write(&path, updated) {
        return format!("Error: {e}");
    }
    "OK".into()
}

pub async fn bash_invoke(ctx: &ToolContext, args_json: &str) -> String {
    let args = match parse_args(args_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let command = match req_str(&args, "command") {
        Ok(s) => s,
        Err(e) => return e,
    };
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    if let Some(cwd) = &ctx.cwd {
        cmd.current_dir(cwd);
    }
    if let Some(extra) = &ctx.extra_env {
        let mut env: HashMap<String, String> = std::env::vars().collect();
        env.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
        cmd.env_clear().envs(env);
    }
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return format!("Error: {e}"),
    };
    match tokio::time::timeout(
        Duration::from_secs(BASH_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    {
        Err(_) => "[timeout]".into(),
        Ok(Err(e)) => format!("Error: {e}"),
        Ok(Ok(output)) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            if output.status.success() {
                text
            } else {
                format!("[exit {}]\n{text}", output.status.code().unwrap_or(1))
            }
        }
    }
}

pub async fn write_invoke(ctx: &ToolContext, args_json: &str) -> String {
    let cwd = ctx.cwd.clone();
    let args_json = args_json.to_string();
    blocking_string(move || write_sync(cwd.as_deref(), &args_json)).await
}

fn write_sync(cwd: Option<&Path>, args_json: &str) -> String {
    let args = match parse_args(args_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let file_path = match req_str(&args, "file_path") {
        Ok(s) => s,
        Err(e) => return e,
    };
    let content = match req_str(&args, "content") {
        Ok(s) => s,
        Err(e) => return e,
    };
    let path = resolve(file_path, cwd);
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return format!("Error: {e}");
        }
    }
    match std::fs::write(&path, content) {
        Ok(()) => "OK".into(),
        Err(e) => format!("Error: {e}"),
    }
}

fn is_binary_prefix(path: &Path) -> bool {
    match std::fs::File::open(path) {
        Ok(mut f) => {
            use std::io::Read;
            let mut buf = [0u8; 8192];
            match f.read(&mut buf) {
                Ok(n) => buf[..n].contains(&0),
                Err(_) => true,
            }
        }
        Err(_) => true,
    }
}

fn fnmatch_name(name: &str, pat: &str) -> bool {
    glob::Pattern::new(pat)
        .map(|p| p.matches(name))
        .unwrap_or(false)
}

fn scan_file(path: &Path, pattern: &Regex, matches: &mut Vec<String>, truncated: &mut bool) {
    if is_binary_prefix(path) {
        return;
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };
    for (i, line) in content.lines().enumerate() {
        if pattern.is_match(line) {
            matches.push(format!("{}:{}:{line}", path.display(), i + 1));
            if matches.len() >= GREP_MAX_LINES {
                *truncated = true;
                return;
            }
        }
    }
}

fn walk_grep(
    dir: &Path,
    pattern: &Regex,
    glob_filter: Option<&str>,
    matches: &mut Vec<String>,
    truncated: &mut bool,
) {
    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };
    entries.sort_by_key(|e| e.file_name());
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for e in entries {
        let name = e.file_name();
        let name_str = name.to_string_lossy();
        let ft = match e.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if ft.is_dir() {
            if name_str.starts_with('.') || SKIP_DIRS.contains(&name_str.as_ref()) {
                continue;
            }
            dirs.push(e.path());
        } else if ft.is_file() || ft.is_symlink() {
            files.push((e.path(), name_str.into_owned()));
        }
    }
    for (path, name) in files {
        if *truncated {
            return;
        }
        if let Some(g) = glob_filter {
            if !fnmatch_name(&name, g) {
                continue;
            }
        }
        scan_file(&path, pattern, matches, truncated);
    }
    for d in dirs {
        if *truncated {
            return;
        }
        walk_grep(&d, pattern, glob_filter, matches, truncated);
    }
}

pub async fn grep_invoke(ctx: &ToolContext, args_json: &str) -> String {
    let cwd = ctx.cwd.clone();
    let args_json = args_json.to_string();
    blocking_string(move || grep_sync(cwd.as_deref(), &args_json)).await
}

fn grep_sync(cwd: Option<&Path>, args_json: &str) -> String {
    let args = match parse_args(args_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let pat = match req_str(&args, "pattern") {
        Ok(s) => s,
        Err(e) => return e,
    };
    let pattern = match Regex::new(pat) {
        Ok(r) => r,
        Err(e) => return format!("Error: invalid regex: {e}"),
    };
    let search_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let base = resolve(search_path, cwd);
    if !base.exists() {
        return format!("Error: path does not exist: {}", base.display());
    }
    let glob_filter = args
        .get("glob")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let mut matches = Vec::new();
    let mut truncated = false;
    if base.is_file() {
        let name = base.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if glob_filter.is_none_or(|g| fnmatch_name(name, g)) {
            if is_binary_prefix(&base) {
                return "(no matches)".into();
            }
            match std::fs::read_to_string(&base) {
                Ok(content) => {
                    for (i, line) in content.lines().enumerate() {
                        if pattern.is_match(line) {
                            matches.push(format!("{}:{}:{line}", base.display(), i + 1));
                            if matches.len() >= GREP_MAX_LINES {
                                truncated = true;
                                break;
                            }
                        }
                    }
                }
                Err(e) => return format!("Error: {e}"),
            }
        }
    } else {
        walk_grep(&base, &pattern, glob_filter, &mut matches, &mut truncated);
    }
    if matches.is_empty() {
        return "(no matches)".into();
    }
    let mut result = matches.join("\n");
    if truncated {
        result.push_str(&format!("\n[truncated at {GREP_MAX_LINES} matches]"));
    }
    result
}

pub async fn glob_invoke(ctx: &ToolContext, args_json: &str) -> String {
    let cwd = ctx.cwd.clone();
    let args_json = args_json.to_string();
    blocking_string(move || glob_sync(cwd.as_deref(), &args_json)).await
}

fn glob_sync(cwd: Option<&Path>, args_json: &str) -> String {
    let args = match parse_args(args_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let pattern = match req_str(&args, "pattern") {
        Ok(s) => s,
        Err(e) => return e,
    };
    let search_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let base = match cwd {
        Some(cwd) => cwd.join(search_path),
        None => PathBuf::from(search_path),
    };
    let full = base.join(pattern);
    let pattern_str = full.to_string_lossy();
    let mut matches: Vec<String> = match glob::glob(&pattern_str) {
        Ok(paths) => paths
            .filter_map(|p| p.ok())
            .map(|p| p.display().to_string())
            .collect(),
        Err(e) => return format!("Error: {e}"),
    };
    matches.sort();
    if matches.is_empty() {
        "(no matches)".into()
    } else {
        matches.join("\n")
    }
}

pub async fn invoke(name: &str, ctx: &ToolContext, args_json: &str) -> String {
    if let Some(allowed) = &ctx.allowed_tools {
        if !allowed.iter().any(|n| n == name) {
            return format!("Error: unknown tool {name}");
        }
    }
    let ka = audit_key_arg(args_json);
    let args = match parse_args(args_json) {
        Ok(v) => v,
        Err(_) => {
            audit(ctx.audit_log.as_deref(), name, &ka, "error", ctx.bypass);
            return "Error: invalid arguments".into();
        }
    };
    if let Some(err) = check_tool(name, ctx, &args) {
        audit(ctx.audit_log.as_deref(), name, &ka, "denied", ctx.bypass);
        return err;
    }
    let res = match name {
        "Read" => read_invoke(ctx, args_json).await,
        "Edit" => edit_invoke(ctx, args_json).await,
        "Bash" => bash_invoke(ctx, args_json).await,
        "Write" => write_invoke(ctx, args_json).await,
        "Grep" => grep_invoke(ctx, args_json).await,
        "Glob" => glob_invoke(ctx, args_json).await,
        other => format!("Error: unknown tool {other}"),
    };
    audit(
        ctx.audit_log.as_deref(),
        name,
        &ka,
        result_status(&res),
        ctx.bypass,
    );
    res
}

pub fn tool_definitions(filter: Option<&[String]>) -> Vec<ToolDefinition> {
    let all = [
        ToolDefinition {
            name: "Read".into(),
            description: "Read a file from the filesystem.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Absolute or relative path"
                    },
                    "limit": {"type": "integer", "description": "Max lines to read"},
                    "offset": {
                        "type": "integer",
                        "description": "Line offset to start from"
                    }
                },
                "required": ["file_path"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "Edit".into(),
            description: "Replace old_string with new_string in a file (first occurrence).".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {"type": "string"},
                    "old_string": {"type": "string"},
                    "new_string": {"type": "string"}
                },
                "required": ["file_path", "old_string", "new_string"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "Bash".into(),
            description: "Run a shell command and return combined stdout/stderr.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"}
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "Write".into(),
            description: "Write content to a file, creating it if necessary.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["file_path", "content"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "Grep".into(),
            description: "Search file contents using a regex pattern.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string"},
                    "path": {
                        "type": "string",
                        "description": "Directory or file to search"
                    },
                    "glob": {"type": "string", "description": "Glob filter for file names"}
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "Glob".into(),
            description: "Find files matching a glob pattern.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string"},
                    "path": {
                        "type": "string",
                        "description": "Base directory to search in"
                    }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        },
    ];
    match filter {
        Some(names) => all
            .into_iter()
            .filter(|t| names.iter().any(|n| n == &t.name))
            .collect(),
        None => all.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "gremlins-tools-{}-{}-{n}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn ctx(cwd: &Path) -> ToolContext {
        ToolContext {
            cwd: Some(cwd.to_path_buf()),
            extra_env: None,
            bypass: false,
            worktree_root: cwd.to_path_buf(),
            audit_log: None,
            allowed_tools: None,
        }
    }

    #[tokio::test]
    async fn read_write_roundtrip() {
        let dir = tmp();
        let c = ctx(&dir);
        let path = dir.join("a.txt");
        let args =
            serde_json::json!({"file_path": path.to_str().unwrap(), "content": "hello\nworld\n"})
                .to_string();
        assert_eq!(write_invoke(&c, &args).await, "OK");
        let read_args = serde_json::json!({"file_path": "a.txt"}).to_string();
        assert_eq!(read_invoke(&c, &read_args).await, "hello\nworld\n");
        let limited =
            serde_json::json!({"file_path": "a.txt", "offset": 1, "limit": 1}).to_string();
        assert_eq!(read_invoke(&c, &limited).await, "world\n");
    }

    #[tokio::test]
    async fn edit_replaces_unique() {
        let dir = tmp();
        let c = ctx(&dir);
        std::fs::write(dir.join("b.txt"), "foo bar foo").unwrap();
        let dup = serde_json::json!({
            "file_path": "b.txt", "old_string": "foo", "new_string": "baz"
        })
        .to_string();
        assert!(edit_invoke(&c, &dup).await.contains("not unique"));
        std::fs::write(dir.join("b.txt"), "foo bar").unwrap();
        let ok = serde_json::json!({
            "file_path": "b.txt", "old_string": "foo", "new_string": "baz"
        })
        .to_string();
        assert_eq!(edit_invoke(&c, &ok).await, "OK");
        assert_eq!(
            std::fs::read_to_string(dir.join("b.txt")).unwrap(),
            "baz bar"
        );
    }

    #[tokio::test]
    async fn bash_echo_and_env() {
        let dir = tmp();
        let mut c = ctx(&dir);
        let args = serde_json::json!({"command": "echo hi"}).to_string();
        assert_eq!(bash_invoke(&c, &args).await.trim(), "hi");
        c.extra_env = Some(HashMap::from([("GREMLIN_TEST_TOKEN".into(), "abc".into())]));
        let env_args =
            serde_json::json!({"command": "printf %s \"$GREMLIN_TEST_TOKEN\""}).to_string();
        assert_eq!(bash_invoke(&c, &env_args).await, "abc");
        let fail = serde_json::json!({"command": "exit 7"}).to_string();
        assert!(bash_invoke(&c, &fail).await.starts_with("[exit 7]"));
    }

    #[tokio::test]
    async fn grep_and_glob() {
        let dir = tmp();
        let c = ctx(&dir);
        std::fs::write(dir.join("one.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();
        std::fs::write(dir.join("two.py"), "def alpha():\n    pass\n").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/three.rs"), "fn alpha() {}\n").unwrap();
        let grep_args = serde_json::json!({"pattern": "alpha", "glob": "*.rs"}).to_string();
        let out = grep_invoke(&c, &grep_args).await;
        assert!(out.contains("one.rs:1:fn alpha() {}"));
        assert!(out.contains("three.rs:1:fn alpha() {}"));
        assert!(!out.contains("two.py"));
        let glob_args = serde_json::json!({"pattern": "**/*.rs"}).to_string();
        let found = glob_invoke(&c, &glob_args).await;
        assert!(found.contains("one.rs"));
        assert!(found.contains("three.rs"));
        assert!(!found.contains("two.py"));
    }

    #[tokio::test]
    async fn grep_no_matches_and_bad_regex() {
        let dir = tmp();
        let c = ctx(&dir);
        std::fs::write(dir.join("z.txt"), "nothing").unwrap();
        let none = serde_json::json!({"pattern": "zzz"}).to_string();
        assert_eq!(grep_invoke(&c, &none).await, "(no matches)");
        let bad = serde_json::json!({"pattern": "["}).to_string();
        assert!(grep_invoke(&c, &bad)
            .await
            .starts_with("Error: invalid regex"));
    }

    #[test]
    fn tool_definitions_filter() {
        let all = tool_definitions(None);
        assert_eq!(all.len(), 6);
        let filtered = tool_definitions(Some(&["Read".into(), "Bash".into()]));
        let names: Vec<_> = filtered.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Read", "Bash"]);
    }

    #[test]
    fn resolve_relative_against_cwd() {
        let cwd = PathBuf::from("/tmp/work");
        assert_eq!(
            resolve("a.txt", Some(&cwd)),
            PathBuf::from("/tmp/work/a.txt")
        );
        assert_eq!(resolve("/abs", Some(&cwd)), PathBuf::from("/abs"));
    }

    #[test]
    fn within_worktree_same_dir() {
        let dir = tmp();
        assert!(within_worktree(&dir, &dir));
    }

    #[test]
    fn within_worktree_child() {
        let dir = tmp();
        assert!(within_worktree(&dir.join("sub").join("file.txt"), &dir));
    }

    #[test]
    fn within_worktree_outside() {
        let dir = tmp();
        assert!(!within_worktree(&dir, &dir.join("sub")));
    }

    #[test]
    fn within_worktree_sibling() {
        let dir = tmp();
        let sibling = dir.parent().unwrap().join("other");
        assert!(!within_worktree(&sibling, &dir));
    }

    #[test]
    #[cfg(unix)]
    fn within_worktree_symlink_traversal() {
        let dir = tmp();
        let worktree = dir.join("worktree");
        std::fs::create_dir(&worktree).unwrap();
        let outside = dir.join("outside");
        std::fs::create_dir(&outside).unwrap();
        let link = worktree.join("link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(!within_worktree(&link, &worktree));
    }

    #[test]
    fn enforce_bypass_allows_outside() {
        let dir = tmp();
        let outside = dir.parent().unwrap().join("other").join("file.txt");
        assert!(enforce(true, &dir, outside.to_str().unwrap(), None).is_none());
    }

    #[test]
    fn enforce_inside_worktree() {
        let dir = tmp();
        let inside = dir.join("file.txt");
        assert!(enforce(false, &dir, inside.to_str().unwrap(), None).is_none());
    }

    #[test]
    fn enforce_outside_worktree() {
        let dir = tmp();
        let outside = dir.parent().unwrap().join("secret.txt");
        let err = enforce(false, &dir, outside.to_str().unwrap(), None).unwrap();
        assert!(err.contains("outside worktree"));
    }

    #[test]
    fn enforce_relative_path_with_cwd() {
        let dir = tmp();
        assert!(enforce(false, &dir, "file.txt", Some(&dir)).is_none());
    }

    #[test]
    fn enforce_relative_path_escapes_via_cwd() {
        let dir = tmp();
        let err = enforce(false, &dir, "../../../etc/passwd", Some(&dir)).unwrap();
        assert!(err.contains("outside worktree"));
    }

    #[test]
    fn bash_check_bypass_allows_absolute() {
        let dir = tmp();
        assert!(bash_check(true, &dir, "cat /etc/passwd", None).is_none());
    }

    #[test]
    fn bash_check_safe_command() {
        let dir = tmp();
        assert!(bash_check(false, &dir, "ls -la", Some(&dir)).is_none());
    }

    #[test]
    fn bash_check_absolute_outside() {
        let dir = tmp();
        let err = bash_check(false, &dir, "cat /etc/passwd", Some(&dir)).unwrap();
        assert!(err.contains("outside worktree"));
    }

    #[test]
    fn bash_check_absolute_inside() {
        let dir = tmp();
        let inside = dir.join("file.txt");
        let cmd = format!("cat {}", inside.display());
        assert!(bash_check(false, &dir, &cmd, Some(&dir)).is_none());
    }

    #[test]
    fn bash_check_tilde_expansion_outside() {
        let dir = tmp();
        let prev = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", "/nonexistent-home") };
        let err = bash_check(false, &dir, "cat ~/.ssh/id_rsa", Some(&dir)).unwrap();
        assert!(err.contains("outside worktree"));
        match prev {
            Some(h) => unsafe { std::env::set_var("HOME", h) },
            None => unsafe { std::env::remove_var("HOME") },
        }
    }

    #[test]
    fn bash_check_dotdot_escapes() {
        let dir = tmp();
        let err = bash_check(false, &dir, "cat ../secret", Some(&dir)).unwrap();
        assert!(err.contains("outside worktree"));
    }

    #[test]
    fn bash_check_intermediate_traversal() {
        let dir = tmp();
        let err = bash_check(false, &dir, "cat subdir/../../etc/passwd", Some(&dir)).unwrap();
        assert!(err.contains("outside worktree"));
    }

    #[test]
    fn bash_check_empty_command() {
        let dir = tmp();
        assert!(bash_check(false, &dir, "", Some(&dir)).is_none());
        assert!(bash_check(false, &dir, "   ", Some(&dir)).is_none());
    }

    #[test]
    fn audit_writes_jsonl() {
        let dir = tmp();
        let log = dir.join("audit.jsonl");
        audit(Some(&log), "Read", "/some/file", "ok", false);
        let lines: Vec<_> = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(|l| l.to_string())
            .collect();
        assert_eq!(lines.len(), 1);
        let entry: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(entry["tool"], "Read");
        assert_eq!(entry["key_arg"], "/some/file");
        assert_eq!(entry["status"], "ok");
        assert_eq!(entry["bypass"], false);
        assert!(entry.get("ts").is_some());
    }

    #[test]
    fn audit_appends_multiple() {
        let dir = tmp();
        let log = dir.join("audit.jsonl");
        audit(Some(&log), "Read", "a", "ok", false);
        audit(Some(&log), "Bash", "b", "denied", false);
        let text = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let entry: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(entry["status"], "denied");
    }

    #[test]
    fn audit_none_log_is_noop() {
        audit(None, "Read", "/file", "ok", false);
    }

    #[test]
    fn audit_truncates_key_arg() {
        let dir = tmp();
        let log = dir.join("audit.jsonl");
        let long_arg = "x".repeat(300);
        audit(Some(&log), "Read", &long_arg, "ok", false);
        let entry: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&log).unwrap()).unwrap();
        assert_eq!(entry["key_arg"].as_str().unwrap().len(), 200);
    }

    #[test]
    fn audit_bypass_flag() {
        let dir = tmp();
        let log = dir.join("audit.jsonl");
        audit(Some(&log), "Bash", "cmd", "ok", true);
        let entry: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&log).unwrap()).unwrap();
        assert_eq!(entry["bypass"], true);
    }

    #[tokio::test]
    async fn invoke_denied_writes_audit() {
        let dir = tmp();
        let log = dir.join("audit.jsonl");
        let mut c = ctx(&dir);
        c.audit_log = Some(log.clone());
        let outside = dir.parent().unwrap().join("secret.txt");
        let args = serde_json::json!({"file_path": outside.to_str().unwrap()}).to_string();
        let result = invoke("Read", &c, &args).await;
        assert!(result.contains("outside worktree"));
        let entry: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&log).unwrap()).unwrap();
        assert_eq!(entry["status"], "denied");
        assert_eq!(entry["tool"], "Read");
    }

    #[tokio::test]
    async fn invoke_ok_writes_audit() {
        let dir = tmp();
        let log = dir.join("audit.jsonl");
        let target = dir.join("hello.txt");
        std::fs::write(&target, "hi").unwrap();
        let mut c = ctx(&dir);
        c.audit_log = Some(log.clone());
        let args = serde_json::json!({"file_path": target.to_str().unwrap()}).to_string();
        let result = invoke("Read", &c, &args).await;
        assert_eq!(result, "hi");
        let entry: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&log).unwrap()).unwrap();
        assert_eq!(entry["status"], "ok");
    }

    #[tokio::test]
    async fn invoke_invalid_json_writes_error() {
        let dir = tmp();
        let log = dir.join("audit.jsonl");
        let mut c = ctx(&dir);
        c.audit_log = Some(log.clone());
        let result = invoke("Read", &c, "not-json{{{").await;
        assert!(result.contains("Error"));
        let entry: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&log).unwrap()).unwrap();
        assert_eq!(entry["status"], "error");
    }

    #[tokio::test]
    async fn invoke_bypass_skips_enforcement() {
        let dir = tmp();
        let worktree = dir.join("worktree");
        std::fs::create_dir(&worktree).unwrap();
        let log = worktree.join("audit.jsonl");
        let outside = dir.join("outside.txt");
        std::fs::write(&outside, "sensitive").unwrap();
        let c = ToolContext {
            cwd: None,
            extra_env: None,
            bypass: true,
            worktree_root: worktree,
            audit_log: Some(log.clone()),
            allowed_tools: None,
        };
        let args = serde_json::json!({"file_path": outside.to_str().unwrap()}).to_string();
        let result = invoke("Read", &c, &args).await;
        assert_eq!(result, "sensitive");
        let entry: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&log).unwrap()).unwrap();
        assert_eq!(entry["status"], "ok");
        assert_eq!(entry["bypass"], true);
    }

    #[tokio::test]
    async fn invoke_bash_denied_writes_audit() {
        let dir = tmp();
        let log = dir.join("audit.jsonl");
        let mut c = ctx(&dir);
        c.audit_log = Some(log.clone());
        let args = serde_json::json!({"command": "cat /etc/passwd"}).to_string();
        let result = invoke("Bash", &c, &args).await;
        assert!(result.contains("outside worktree"));
        let entry: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&log).unwrap()).unwrap();
        assert_eq!(entry["status"], "denied");
        assert_eq!(entry["tool"], "Bash");
    }

    #[tokio::test]
    async fn filtered_tool_does_not_mutate_or_spawn() {
        let dir = tmp();
        let target = dir.join("must-not-exist.txt");
        let c = ToolContext {
            cwd: Some(dir.clone()),
            extra_env: None,
            bypass: true,
            worktree_root: dir.clone(),
            audit_log: None,
            allowed_tools: Some(vec!["Read".into()]),
        };
        let write_args =
            serde_json::json!({"file_path": target.to_str().unwrap(), "content": "nope"})
                .to_string();
        let write_out = invoke("Write", &c, &write_args).await;
        assert!(write_out.contains("unknown tool"));
        assert!(!target.exists());

        let marker = dir.join("bash-ran");
        let bash_args = serde_json::json!({
            "command": format!("touch {}", marker.display())
        })
        .to_string();
        let bash_out = invoke("Bash", &c, &bash_args).await;
        assert!(bash_out.contains("unknown tool"));
        assert!(!marker.exists());
    }

    #[test]
    fn audit_log_path_sibling() {
        let raw = PathBuf::from("/tmp/run.jsonl");
        assert_eq!(audit_log_path(&raw), PathBuf::from("/tmp/run.audit.jsonl"));
    }
}
