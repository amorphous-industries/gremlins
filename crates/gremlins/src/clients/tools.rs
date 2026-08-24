use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use regex::Regex;
use rig_core::completion::ToolDefinition;
use tokio::process::Command;

const GREP_MAX_LINES: usize = 2000;
const BASH_TIMEOUT_SECS: u64 = 120;
const SKIP_DIRS: &[&str] = &["__pycache__", "node_modules", "target"];

type SubagentFuture = Pin<Box<dyn std::future::Future<Output = String> + Send>>;

/// Callback that `invoke` calls for subagent tool invocations.
pub type SubagentFn = Arc<dyn Fn(String, Option<PathBuf>) -> SubagentFuture + Send + Sync>;

#[derive(Clone)]
pub struct ToolContext {
    pub cwd: Option<PathBuf>,
    pub extra_env: Option<HashMap<String, String>>,
    pub worktree_root: PathBuf,
    pub audit_log: Option<PathBuf>,
    pub allowed_tools: Option<Vec<String>>,
    pub subagent_fn: Option<SubagentFn>,
    pub audit_lock: Option<Arc<std::sync::Mutex<()>>>,
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

/// Resolve the real path by canonicalizing the deepest ancestor that exists
/// on disk, then re-appending the non-existent suffix. Falls back to the
/// original path if even the filesystem root can't be canonicalized.
fn canonicalize_or_ancestor(p: &Path) -> PathBuf {
    match p.canonicalize() {
        Ok(c) => c,
        Err(_) => {
            let mut ancestor = p;
            loop {
                match ancestor.canonicalize() {
                    Ok(c) => {
                        let suffix = p.strip_prefix(ancestor).unwrap_or(p);
                        // Resolve dangling symlinks in the suffix.
                        // canonicalize() walks past symlinks whose target
                        // doesn't exist yet. A dangling symlink pointing
                        // outside would pass containment as an ordinary
                        // name. Check each suffix prefix; if we find a
                        // symlink, resolve it and recurse so the real
                        // target is seen by the containment check.
                        let components: Vec<_> = suffix.components().collect();
                        let mut probe = c.clone();
                        for (i, comp) in components.iter().enumerate() {
                            probe = probe.join(comp);
                            if probe.is_symlink() {
                                if let Ok(target) = std::fs::read_link(&probe) {
                                    let mut resolved = if target.is_absolute() {
                                        target
                                    } else {
                                        probe.parent().unwrap().join(&target)
                                    };
                                    for comp in &components[i + 1..] {
                                        resolved = resolved.join(comp);
                                    }
                                    return canonicalize_or_ancestor(&resolved);
                                }
                                break;
                            }
                        }
                        return c.join(suffix);
                    }
                    Err(_) => match ancestor.parent() {
                        Some(parent) if !parent.as_os_str().is_empty() => ancestor = parent,
                        _ => return p.to_path_buf(),
                    },
                }
            }
        }
    }
}

fn normalize_path(p: &Path) -> Option<PathBuf> {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(p)
    };
    let canonical = canonicalize_or_ancestor(&abs);
    // normalize_dots handles `.`/`..` in the non-existent suffix only —
    // canonicalize_or_ancestor already resolved them in the existing prefix.
    // Lexical `..` in a not-yet-existent suffix is the safe conservative choice.
    Some(normalize_dots(&canonical))
}

pub fn within_worktree(p: &Path, root: &Path) -> bool {
    match (normalize_path(p), normalize_path(root)) {
        (Some(p), Some(root)) => p.starts_with(root),
        _ => false,
    }
}

/// Hot-path variant: caller has already canonicalized root via [`normalize_path`].
/// Avoids re-canonicalizing root on every call inside loops like [`bash_check`].
fn within_worktree_precanon(p: &Path, canonical_root: &Path) -> bool {
    match normalize_path(p) {
        Some(p) => p.starts_with(canonical_root),
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

pub fn enforce(root: &Path, pth: &str, cwd: Option<&Path>) -> Option<String> {
    let p = resolve(pth, cwd);
    if !within_worktree(&p, root) {
        return Some(format!("Error: path outside worktree: {pth}"));
    }
    None
}

/// Symlink-aware containment check for file I/O. Uses
/// [`canonicalize_or_ancestor`] to resolve the real path, including dangling
/// symlinks that would otherwise escape containment. Returns Some(error) or
/// None if the path is safe.
pub fn io_enforce(path: &Path, root: &Path) -> Option<String> {
    let real_root = match root.canonicalize() {
        Ok(c) => c,
        Err(_) => return None, // paranoid, shouldn't happen for real worktree
    };
    let real = canonicalize_or_ancestor(path);
    if !real.starts_with(&real_root) {
        return Some(format!(
            "Error: path outside worktree (resolved): {}",
            real.display()
        ));
    }
    None
}

/// Returns Some(error) if `cmd` is an `ln` invocation that requests
/// symbolic linking. Token-based: inspects argv[0] for `ln` and scans
/// subsequent tokens for `-s*`, `--symbolic`, or flags containing `s`.
///
/// Defense-in-depth only. `io_enforce` is the real containment boundary:
/// this misses `env ln -s`, `command ln -s`, `sh -c 'ln -s'`, and any `ln`
/// after `;`/`&&`/`|`, since it only inspects argv[0]. It exists to give the
/// model a clear early denial for the common `ln -s` case, not to be airtight.
fn check_ln_symlink(cmd: &str) -> Option<String> {
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    let first = tokens.first()?.trim_matches(|c| c == '\'' || c == '"');
    // Match "ln" or paths ending in "/ln" (e.g. /bin/ln).
    if first != "ln" && !first.ends_with("/ln") {
        return None;
    }
    for tok in &tokens[1..] {
        let stripped = tok.trim_matches(|c| c == '\'' || c == '"');
        if stripped == "--symbolic" || stripped.starts_with("-s") {
            return Some("Error: creating symlinks is not allowed".into());
        }
        // Catch short-flag bundles like -sf, -fs, -sn. Any short flag
        // containing 's' is rejected. This still has false positives
        // (-ns, --version), but those are harmless denials in practice.
        if stripped.starts_with('-') && stripped.len() > 1 && stripped.contains('s') {
            return Some("Error: creating symlinks is not allowed".into());
        }
    }
    None
}

pub fn bash_check(root: &Path, cmd: &str, cwd: Option<&Path>) -> Option<String> {
    let s = cmd.trim();
    if s.is_empty() {
        return None;
    }
    // Guard against symlink creation via ln.
    if let Some(err) = check_ln_symlink(s) {
        return Some(err);
    }
    // Canonicalize root once — avoid re-resolving per token.
    let canonical_root = match normalize_path(root) {
        Some(cr) => cr,
        _ => return Some("Error: invalid worktree root".into()),
    };
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
        if !within_worktree_precanon(&p, &canonical_root) {
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

fn audit(
    log: Option<&Path>,
    lock: Option<&std::sync::Mutex<()>>,
    tool: &str,
    key_arg: &str,
    status: &str,
) {
    let Some(log) = log else {
        return;
    };
    let truncated: String = key_arg.chars().take(200).collect();
    let entry = serde_json::json!({
        "ts": super::stream::ts_internal(),
        "tool": tool,
        "key_arg": truncated,
        "status": status,
    });
    let Ok(line) = serde_json::to_string(&entry) else {
        return;
    };
    if let Some(lock) = lock {
        // audit is best-effort; skip write if the mutex is poisoned rather
        // than panicking and crashing the tool invocation.
        if let Ok(_guard) = lock.lock() {
            audit_write(log, &line);
        }
    } else {
        audit_write(log, &line);
    }
}

fn audit_write(log: &Path, line: &str) {
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
            &ctx.worktree_root,
            args.get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("."),
            ctx.cwd.as_deref(),
        ),
        "Grep" => enforce(
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
            enforce(&ctx.worktree_root, &full_str, None)
        }
        "Bash" => bash_check(
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
    let root = ctx.worktree_root.clone();
    let args_json = args_json.to_string();
    blocking_string(move || read_sync(cwd.as_deref(), &root, &args_json)).await
}

fn read_sync(cwd: Option<&Path>, root: &Path, args_json: &str) -> String {
    let args = match parse_args(args_json) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let file_path = match req_str(&args, "file_path") {
        Ok(s) => s,
        Err(e) => return e,
    };
    let path = resolve(file_path, cwd);
    if let Some(err) = io_enforce(&path, root) {
        return err;
    }
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
    let root = ctx.worktree_root.clone();
    let args_json = args_json.to_string();
    blocking_string(move || edit_sync(cwd.as_deref(), &root, &args_json)).await
}

fn edit_sync(cwd: Option<&Path>, root: &Path, args_json: &str) -> String {
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
    if let Some(err) = io_enforce(&path, root) {
        return err;
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => return format!("Error: {e}"),
    };
    if old.is_empty() {
        return format!("Error: old_string is empty in {file_path}");
    }
    if !content.contains(old) {
        let first_line = old.lines().next().unwrap_or("").trim();
        // Char-boundary-safe truncation to 80 characters.
        let needle: &str = if first_line.is_empty() {
            // Don't use an empty needle — find() matches everywhere.
            return format!(
                "Error: old_string not found in {file_path} — first line is empty or whitespace-only"
            );
        } else if first_line.chars().count() > 80 {
            let byte_idx = first_line
                .char_indices()
                .nth(80)
                .map(|(i, _)| i)
                .unwrap_or(first_line.len());
            &first_line[..byte_idx]
        } else {
            first_line
        };
        let hint = if let Some(pos) = content.find(needle) {
            let line_no = content[..pos].lines().count() + 1;
            let context_start = content[..pos].rfind('\n').map_or(0, |n| n + 1);
            let context_end = content[pos..].find('\n').map_or(content.len(), |n| pos + n);
            // Context capped at 200 chars, clamped to a char boundary.
            let cap_byte = content[context_start..]
                .char_indices()
                .nth(200)
                .map(|(i, _)| context_start + i)
                .unwrap_or(content.len());
            let context_end = context_end.min(cap_byte);
            let context = &content[context_start..context_end];
            format!(" — did you mean to match near line {line_no}? Found:\n{context}")
        } else {
            String::new()
        };
        return format!(
            "Error: old_string not found in {file_path} — first line: {needle:?}{hint}"
        );
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
    let root = ctx.worktree_root.clone();
    let args_json = args_json.to_string();
    blocking_string(move || write_sync(cwd.as_deref(), &root, &args_json)).await
}

fn write_sync(cwd: Option<&Path>, root: &Path, args_json: &str) -> String {
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
    if let Some(err) = io_enforce(&path, root) {
        return err;
    }
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

fn scan_file(
    path: &Path,
    root: &Path,
    pattern: &Regex,
    matches: &mut Vec<String>,
    truncated: &mut bool,
) {
    // Skip files that resolve outside the worktree (e.g. via a symlink).
    if io_enforce(path, root).is_some() {
        return;
    }
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
    root: &Path,
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
        scan_file(&path, root, pattern, matches, truncated);
    }
    for d in dirs {
        if *truncated {
            return;
        }
        // Skip directory symlinks that resolve outside the worktree.
        if io_enforce(&d, root).is_some() {
            continue;
        }
        walk_grep(&d, root, pattern, glob_filter, matches, truncated);
    }
}

pub async fn grep_invoke(ctx: &ToolContext, args_json: &str) -> String {
    let cwd = ctx.cwd.clone();
    let root = ctx.worktree_root.clone();
    let args_json = args_json.to_string();
    blocking_string(move || grep_sync(cwd.as_deref(), &root, &args_json)).await
}

fn grep_sync(cwd: Option<&Path>, root: &Path, args_json: &str) -> String {
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
        if let Some(err) = io_enforce(&base, root) {
            return err;
        }
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
        walk_grep(
            &base,
            root,
            &pattern,
            glob_filter,
            &mut matches,
            &mut truncated,
        );
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
    let root = ctx.worktree_root.clone();
    let args_json = args_json.to_string();
    blocking_string(move || glob_sync(cwd.as_deref(), &root, &args_json)).await
}

fn glob_sync(cwd: Option<&Path>, root: &Path, args_json: &str) -> String {
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
            // Drop matches that resolve outside the worktree (e.g. via a symlink).
            .filter(|p| io_enforce(p, root).is_none())
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
        // subagent is always permitted regardless of tool filter.
        if name != "subagent" && !allowed.iter().any(|n| n == name) {
            return format!("Error: unknown tool {name}");
        }
    }
    let ka = audit_key_arg(args_json);
    let args = match parse_args(args_json) {
        Ok(v) => v,
        Err(_) => {
            audit(
                ctx.audit_log.as_deref(),
                ctx.audit_lock.as_deref(),
                name,
                &ka,
                "error",
            );
            return "Error: invalid arguments".into();
        }
    };
    if let Some(err) = check_tool(name, ctx, &args) {
        audit(
            ctx.audit_log.as_deref(),
            ctx.audit_lock.as_deref(),
            name,
            &ka,
            "denied",
        );
        return err;
    }
    let res = match name {
        "Read" => read_invoke(ctx, args_json).await,
        "Edit" => edit_invoke(ctx, args_json).await,
        "Bash" => bash_invoke(ctx, args_json).await,
        "Write" => write_invoke(ctx, args_json).await,
        "Grep" => grep_invoke(ctx, args_json).await,
        "Glob" => glob_invoke(ctx, args_json).await,
        "subagent" => {
            if let Some(f) = &ctx.subagent_fn {
                let task = args.get("task").and_then(|v| v.as_str()).unwrap_or("");
                if task.is_empty() {
                    return "Error: subagent task is required".to_string();
                }
                let cwd = args.get("cwd").and_then(|v| v.as_str()).map(PathBuf::from);
                f(task.to_string(), cwd).await
            } else {
                "Error: subagent not available for this backend".to_string()
            }
        }
        other => format!("Error: unknown tool {other}"),
    };
    audit(
        ctx.audit_log.as_deref(),
        ctx.audit_lock.as_deref(),
        name,
        &ka,
        result_status(&res),
    );
    res
}

pub fn tool_definitions(filter: Option<&[String]>) -> Vec<ToolDefinition> {
    let mut all = vec![
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
    // Subagent is always available, even when a tool filter is set.
    all.push(ToolDefinition {
        name: "subagent".into(),
        description: "Delegate a single task to a nested agent that runs with a clean conversation context but the same worktree and tools as you. Provide the task in `task`. The subagent inherits your working directory by default; pass `cwd` to run it elsewhere within the worktree. Returns the subagent's final text output.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Task description for the subagent"
                },
                "cwd": {
                    "type": "string",
                    "description": "Optional working directory override"
                }
            },
            "required": ["task"],
            "additionalProperties": false
        }),
    });
    match filter {
        Some(names) => all
            .into_iter()
            .filter(|t| t.name == "subagent" || names.iter().any(|n| n == &t.name))
            .collect(),
        None => all,
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
            worktree_root: cwd.to_path_buf(),
            audit_log: None,
            allowed_tools: None,
            subagent_fn: None,
            audit_lock: None,
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
    async fn bash_invoke_respects_cwd() {
        // Reproduction: pwd must report cwd, not the process working directory.
        let dir = tmp();
        let marker = "gremlins-cwd-test-marker";
        std::fs::write(dir.join(marker), "ok").unwrap();
        let c = ToolContext {
            cwd: Some(dir.clone()),
            extra_env: None,
            worktree_root: dir.clone(),
            audit_log: None,
            allowed_tools: None,
            subagent_fn: None,
            audit_lock: None,
        };
        let args = serde_json::json!({"command": "pwd; ls"}).to_string();
        let output = bash_invoke(&c, &args).await;
        assert!(
            output.contains(dir.to_str().unwrap()),
            "bash_invoke pwd must contain cwd={}, got: {output}",
            dir.display()
        );
        assert!(
            output.contains(marker),
            "bash_invoke ls must see marker file '{marker}', got: {output}"
        );
    }

    #[tokio::test]
    async fn bash_invoke_cwd_none_uses_process_cwd() {
        // When cwd is None, the command inherits the process current directory.
        let c = ToolContext {
            cwd: None,
            extra_env: None,
            worktree_root: std::env::current_dir().unwrap(),
            audit_log: None,
            allowed_tools: None,
            subagent_fn: None,
            audit_lock: None,
        };
        let expected = std::env::current_dir().unwrap();
        let args = serde_json::json!({"command": "pwd"}).to_string();
        let output = bash_invoke(&c, &args).await;
        assert!(
            output.trim().contains(expected.to_str().unwrap()),
            "bash_invoke with cwd=None should use process cwd={}, got: {output}",
            expected.display()
        );
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
    #[cfg(unix)]
    async fn grep_and_glob_skip_symlink_escape() {
        let root = tmp();
        let worktree = root.join("worktree");
        std::fs::create_dir(&worktree).unwrap();
        let outside = root.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("secret.rs"), "fn alpha() {}\n").unwrap();
        // Symlinked file and directory inside the worktree pointing out.
        std::os::unix::fs::symlink(outside.join("secret.rs"), worktree.join("leak.rs")).unwrap();
        std::os::unix::fs::symlink(&outside, worktree.join("outdir")).unwrap();
        // A legitimate in-worktree file that should still be found.
        std::fs::write(worktree.join("ok.rs"), "fn alpha() {}\n").unwrap();

        let c = ctx(&worktree);
        let grep_args = serde_json::json!({"pattern": "alpha"}).to_string();
        let out = grep_invoke(&c, &grep_args).await;
        assert!(
            out.contains("ok.rs"),
            "in-worktree file should match: {out}"
        );
        assert!(
            !out.contains("leak.rs"),
            "symlinked file must be skipped: {out}"
        );
        assert!(
            !out.contains("secret.rs"),
            "escaped dir must be skipped: {out}"
        );

        let glob_args = serde_json::json!({"pattern": "**/*.rs"}).to_string();
        let found = glob_invoke(&c, &glob_args).await;
        assert!(
            found.contains("ok.rs"),
            "in-worktree file should glob: {found}"
        );
        assert!(
            !found.contains("leak.rs"),
            "symlinked file must be filtered: {found}"
        );
        assert!(
            !found.contains("secret.rs"),
            "escaped dir must be filtered: {found}"
        );
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
        assert_eq!(all.len(), 7);
        let filtered = tool_definitions(Some(&["Read".into(), "Bash".into()]));
        let names: Vec<_> = filtered.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Read", "Bash", "subagent"]);
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
        // Canonical containment: symlink resolves outside the worktree.
        assert!(!within_worktree(&link, &worktree));
    }

    #[test]
    fn enforce_inside_worktree() {
        let dir = tmp();
        let inside = dir.join("file.txt");
        assert!(enforce(&dir, inside.to_str().unwrap(), None).is_none());
    }

    #[test]
    fn enforce_outside_worktree() {
        let dir = tmp();
        let outside = dir.parent().unwrap().join("secret.txt");
        let err = enforce(&dir, outside.to_str().unwrap(), None).unwrap();
        assert!(err.contains("outside worktree"));
    }

    #[test]
    fn enforce_relative_path_with_cwd() {
        let dir = tmp();
        assert!(enforce(&dir, "file.txt", Some(&dir)).is_none());
    }

    #[test]
    fn enforce_relative_path_escapes_via_cwd() {
        let dir = tmp();
        let err = enforce(&dir, "../../../etc/passwd", Some(&dir)).unwrap();
        assert!(err.contains("outside worktree"));
    }

    #[test]
    fn bash_check_safe_command() {
        let dir = tmp();
        assert!(bash_check(&dir, "ls -la", Some(&dir)).is_none());
    }

    #[test]
    fn bash_check_absolute_outside() {
        let dir = tmp();
        let err = bash_check(&dir, "cat /etc/passwd", Some(&dir)).unwrap();
        assert!(err.contains("outside worktree"));
    }

    #[test]
    fn bash_check_absolute_inside() {
        let dir = tmp();
        let inside = dir.join("file.txt");
        let cmd = format!("cat {}", inside.display());
        assert!(bash_check(&dir, &cmd, Some(&dir)).is_none());
    }

    #[test]
    fn bash_check_tilde_expansion_outside() {
        let dir = tmp();
        let prev = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", "/nonexistent-home") };
        let err = bash_check(&dir, "cat ~/.ssh/id_rsa", Some(&dir)).unwrap();
        assert!(err.contains("outside worktree"));
        match prev {
            Some(h) => unsafe { std::env::set_var("HOME", h) },
            None => unsafe { std::env::remove_var("HOME") },
        }
    }

    #[test]
    fn bash_check_dotdot_escapes() {
        let dir = tmp();
        let err = bash_check(&dir, "cat ../secret", Some(&dir)).unwrap();
        assert!(err.contains("outside worktree"));
    }

    #[test]
    fn bash_check_intermediate_traversal() {
        let dir = tmp();
        let err = bash_check(&dir, "cat subdir/../../etc/passwd", Some(&dir)).unwrap();
        assert!(err.contains("outside worktree"));
    }

    #[test]
    fn bash_check_empty_command() {
        let dir = tmp();
        assert!(bash_check(&dir, "", Some(&dir)).is_none());
        assert!(bash_check(&dir, "   ", Some(&dir)).is_none());
    }

    #[test]
    fn audit_writes_jsonl() {
        let dir = tmp();
        let log = dir.join("audit.jsonl");
        audit(Some(&log), None, "Read", "/some/file", "ok");
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
        assert!(entry.get("ts").is_some());
    }

    #[test]
    fn audit_appends_multiple() {
        let dir = tmp();
        let log = dir.join("audit.jsonl");
        audit(Some(&log), None, "Read", "a", "ok");
        audit(Some(&log), None, "Bash", "b", "denied");
        let text = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let entry: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(entry["status"], "denied");
    }

    #[test]
    fn audit_none_log_is_noop() {
        audit(None, None, "Read", "/file", "ok");
    }

    #[test]
    fn audit_truncates_key_arg() {
        let dir = tmp();
        let log = dir.join("audit.jsonl");
        let long_arg = "x".repeat(300);
        audit(Some(&log), None, "Read", &long_arg, "ok");
        let entry: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&log).unwrap()).unwrap();
        assert_eq!(entry["key_arg"].as_str().unwrap().len(), 200);
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
            worktree_root: dir.clone(),
            audit_log: None,
            allowed_tools: Some(vec!["Read".into()]),
            subagent_fn: None,
            audit_lock: None,
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

    // --- Part 1: Lexical containment tests ---

    #[test]
    fn bash_check_blocks_ln_s_flag() {
        let dir = tmp();
        let err = bash_check(&dir, "ln -s /tmp/foo bar", Some(&dir)).unwrap();
        assert!(err.contains("symlinks is not allowed"));
    }

    #[test]
    fn bash_check_blocks_ln_sf_flag() {
        let dir = tmp();
        let err = bash_check(&dir, "ln -sf /tmp/foo bar", Some(&dir)).unwrap();
        assert!(err.contains("symlinks is not allowed"));
    }

    #[test]
    fn bash_check_blocks_ln_symbolic_flag() {
        let dir = tmp();
        let err = bash_check(&dir, "ln --symbolic foo bar", Some(&dir)).unwrap();
        assert!(err.contains("symlinks is not allowed"));
    }

    #[test]
    fn bash_check_blocks_bin_ln_s() {
        let dir = tmp();
        let err = bash_check(&dir, "/bin/ln -s foo bar", Some(&dir)).unwrap();
        assert!(err.contains("symlinks is not allowed"));
    }

    #[test]
    fn bash_check_allows_column_s() {
        let dir = tmp();
        // "column -s , file.csv" is not an ln invocation.
        assert!(bash_check(&dir, "column -s , file.csv", Some(&dir)).is_none());
    }

    #[test]
    fn bash_check_allows_echo_ln_s() {
        let dir = tmp();
        // echo "ln -s" is an echo, not ln.
        assert!(bash_check(&dir, "echo \"ln -s\"", Some(&dir)).is_none());
    }

    #[test]
    #[cfg(unix)]
    fn enforce_rejects_symlink_escape() {
        let dir = tmp();
        let worktree = dir.join("worktree");
        std::fs::create_dir(&worktree).unwrap();
        let outside = dir.join("outside");
        std::fs::create_dir(&outside).unwrap();
        let link = worktree.join("link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        // Canonical containment: symlink resolves outside the worktree.
        let err = enforce(&worktree, link.to_str().unwrap(), None).unwrap();
        assert!(err.contains("outside worktree"));
    }

    #[test]
    #[cfg(unix)]
    fn enforce_rejects_dangling_symlink_escape() {
        let dir = tmp();
        let worktree = dir.join("worktree");
        std::fs::create_dir(&worktree).unwrap();
        let outside = dir.join("outside");
        std::fs::create_dir(&outside).unwrap();
        // Dangling symlink: target doesn't exist, so canonicalize() walks
        // past it. The containment check must still read_link() and reject.
        let link = worktree.join("link");
        std::os::unix::fs::symlink(outside.join("new.txt"), &link).unwrap();
        let err = enforce(&worktree, link.to_str().unwrap(), None).unwrap();
        assert!(err.contains("outside worktree"), "got: {err}");
    }

    #[test]
    fn within_worktree_lexical_child() {
        let dir = tmp();
        let child = dir.join("sub").join("file.txt");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(&child, "x").unwrap();
        assert!(within_worktree(&child, &dir));
    }

    // --- Part 2: Edit diagnostics tests ---

    #[test]
    fn edit_not_found_shows_first_line() {
        let dir = tmp();
        std::fs::write(dir.join("f.txt"), "hello\nworld\n").unwrap();
        let args = serde_json::json!({
            "file_path": "f.txt",
            "old_string": "fn alpha() {}",
            "new_string": ""
        })
        .to_string();
        let result = edit_sync(Some(&dir), &dir, &args);
        assert!(result.contains("Error: old_string not found"));
        assert!(result.contains("fn alpha() {}"));
    }

    #[test]
    fn edit_not_found_shows_line_hint() {
        let dir = tmp();
        std::fs::write(dir.join("f.txt"), "hello world\nfn alpha() {}\n").unwrap();
        let args = serde_json::json!({
            "file_path": "f.txt",
            "old_string": "fn alpha() {\n  x = 1\n}",
            "new_string": ""
        })
        .to_string();
        let result = edit_sync(Some(&dir), &dir, &args);
        assert!(result.contains("did you mean to match"));
        assert!(result.contains("fn alpha"));
    }

    #[test]
    fn edit_empty_old_string() {
        let dir = tmp();
        std::fs::write(dir.join("f.txt"), "hello\n").unwrap();
        let args = serde_json::json!({
            "file_path": "f.txt",
            "old_string": "",
            "new_string": ""
        })
        .to_string();
        let result = edit_sync(Some(&dir), &dir, &args);
        assert!(result.contains("old_string is empty"));
    }

    #[test]
    fn edit_not_found_whitespace_only_first_line() {
        let dir = tmp();
        std::fs::write(dir.join("f.txt"), "hello\n").unwrap();
        // old_string starts with a blank line, so first_line trims to empty.
        let args = serde_json::json!({
            "file_path": "f.txt",
            "old_string": "\n  fn alpha() {}",
            "new_string": ""
        })
        .to_string();
        let result = edit_sync(Some(&dir), &dir, &args);
        assert!(result.contains("first line is empty"));
    }

    #[test]
    fn edit_not_found_multibyte_truncation() {
        let dir = tmp();
        std::fs::write(dir.join("f.txt"), "hello\n").unwrap();
        // First line > 80 chars with multibyte content — must not panic.
        let long_line = "é".repeat(100);
        let old = format!("{long_line}\nbody");
        let args = serde_json::json!({
            "file_path": "f.txt",
            "old_string": old,
            "new_string": ""
        })
        .to_string();
        let result = edit_sync(Some(&dir), &dir, &args);
        assert!(result.contains("old_string not found"));
        // The needle should be truncated to 80 chars, not 80 bytes.
        // 100 é's is 200 bytes, so untruncated needle would be 200 bytes.
        assert!(!result.contains(&long_line));
    }

    #[test]
    fn edit_not_found_multibyte_context() {
        let dir = tmp();
        // File content with multibyte characters near the match.
        let content = "é".repeat(300) + "\nfn alpha() {}\nbar";
        std::fs::write(dir.join("f.txt"), &content).unwrap();
        let args = serde_json::json!({
            "file_path": "f.txt",
            "old_string": "fn alpha() {\n  x = 1\n}",
            "new_string": ""
        })
        .to_string();
        let result = edit_sync(Some(&dir), &dir, &args);
        assert!(result.contains("did you mean to match"));
        // Must not panic on byte-slice boundary.
    }

    // --- Part 3: Subagent tests ---

    #[tokio::test]
    async fn subagent_no_callback_returns_error() {
        let dir = tmp();
        let c = ToolContext {
            cwd: Some(dir.clone()),
            extra_env: None,
            worktree_root: dir.clone(),
            audit_log: None,
            allowed_tools: None,
            subagent_fn: None,
            audit_lock: None,
        };
        let args = serde_json::json!({"task": "do something"}).to_string();
        let result = invoke("subagent", &c, &args).await;
        assert!(result.contains("subagent not available"));
    }

    #[tokio::test]
    async fn subagent_callback_called_when_set() {
        let dir = tmp();
        let called = Arc::new(std::sync::Mutex::new(false));
        let called2 = called.clone();
        let subagent_fn: SubagentFn = Arc::new(move |task, cwd| {
            *called2.lock().unwrap() = true;
            Box::pin(async move { format!("subagent result: {task} cwd={cwd:?}") })
        });
        let c = ToolContext {
            cwd: Some(dir.clone()),
            extra_env: None,
            worktree_root: dir.clone(),
            audit_log: None,
            allowed_tools: None,
            subagent_fn: Some(subagent_fn),
            audit_lock: None,
        };
        let args = serde_json::json!({"task": "do something", "cwd": "/tmp/x"}).to_string();
        let result = invoke("subagent", &c, &args).await;
        assert!(*called.lock().unwrap());
        assert!(result.contains("subagent result"));
        assert!(result.contains("/tmp/x"));
    }

    #[tokio::test]
    async fn subagent_empty_task_returns_error() {
        let dir = tmp();
        let subagent_fn: SubagentFn =
            Arc::new(|_task, _cwd| Box::pin(async move { "should not be called".to_string() }));
        let c = ToolContext {
            cwd: Some(dir.clone()),
            extra_env: None,
            worktree_root: dir,
            audit_log: None,
            allowed_tools: None,
            subagent_fn: Some(subagent_fn),
            audit_lock: None,
        };
        // Empty task string.
        let args = serde_json::json!({"task": ""}).to_string();
        let result = invoke("subagent", &c, &args).await;
        assert!(result.contains("subagent task is required"));

        // Missing task key entirely.
        let args2 = serde_json::json!({}).to_string();
        let result2 = invoke("subagent", &c, &args2).await;
        assert!(result2.contains("subagent task is required"));
    }

    // --- Part 4: IO containment tests (symlink-aware) ---

    #[test]
    fn io_enforce_passes_for_lexical_child() {
        let dir = tmp();
        let f = dir.join("a.txt");
        std::fs::write(&f, "hi").unwrap();
        assert!(io_enforce(&f, &dir).is_none());
    }

    #[test]
    fn io_enforce_passes_for_new_file() {
        let dir = tmp();
        let f = dir.join("new.txt");
        // Non-existent file passes — can't be a bad symlink if it doesn't exist.
        assert!(io_enforce(&f, &dir).is_none());
    }

    #[test]
    fn io_enforce_passes_for_lexical_sibling() {
        let dir = tmp();
        let sibling = dir.parent().unwrap().join(format!(
            "gremlins-io-sibling-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&sibling).unwrap();
        let target = sibling.join("out.txt");
        std::fs::write(&target, "x").unwrap();
        // target is outside the worktree (sibling, not child).
        let err = io_enforce(&target, &dir).unwrap();
        assert!(err.contains("outside worktree"), "got: {err}");
    }

    #[test]
    #[cfg(unix)]
    fn io_enforce_blocks_symlink_escape() {
        let dir = tmp();
        let worktree = dir.join("worktree");
        std::fs::create_dir(&worktree).unwrap();
        let outside = dir.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "sensitive").unwrap();
        let link = worktree.join("link.txt");
        std::os::unix::fs::symlink(outside.join("secret.txt"), &link).unwrap();
        // Lexically inside the worktree, but symlink resolves outside.
        let err = io_enforce(&link, &worktree).unwrap();
        assert!(err.contains("outside worktree"), "got: {err}");
    }

    #[test]
    #[cfg(unix)]
    fn io_enforce_blocks_write_through_symlinked_parent() {
        let dir = tmp();
        let worktree = dir.join("worktree");
        std::fs::create_dir(&worktree).unwrap();
        let outside = dir.join("outside");
        std::fs::create_dir(&outside).unwrap();
        // A pre-existing symlinked directory inside the worktree pointing out.
        let link_dir = worktree.join("venv-link");
        std::os::unix::fs::symlink(&outside, &link_dir).unwrap();
        // The leaf doesn't exist yet, but its parent resolves outside.
        let target = link_dir.join("newfile.txt");
        let err = io_enforce(&target, &worktree).unwrap();
        assert!(err.contains("outside worktree"), "got: {err}");
    }

    #[test]
    #[cfg(unix)]
    fn io_enforce_allows_new_file_under_in_worktree_symlink() {
        let dir = tmp();
        let worktree = dir.join("worktree");
        std::fs::create_dir(&worktree).unwrap();
        let real = worktree.join("real");
        std::fs::create_dir(&real).unwrap();
        // A symlink that stays inside the worktree is fine.
        let link_dir = worktree.join("link");
        std::os::unix::fs::symlink(&real, &link_dir).unwrap();
        let target = link_dir.join("newfile.txt");
        assert!(io_enforce(&target, &worktree).is_none());
    }

    #[test]
    #[cfg(unix)]
    fn io_enforce_blocks_dangling_symlink_escape() {
        let dir = tmp();
        let worktree = dir.join("worktree");
        std::fs::create_dir(&worktree).unwrap();
        let outside = dir.join("outside");
        std::fs::create_dir(&outside).unwrap();
        // Dangling symlink: target doesn't exist yet, so canonicalize()
        // fails on the symlink itself. We must still resolve the symlink
        // target for containment.
        let link = worktree.join("link");
        std::os::unix::fs::symlink(outside.join("new.txt"), &link).unwrap();
        let err = io_enforce(&link, &worktree).unwrap();
        assert!(err.contains("outside worktree"), "got: {err}");
    }
}
