use std::collections::HashSet;
use std::path::PathBuf;

pub mod error;
pub use error::DiscoveryError;

use crate::assets::{PIPELINES, PIPELINE_PATHS};

fn project_overlay_dir(project_root: &std::path::Path) -> PathBuf {
    if let Ok(ov) = std::env::var("GREMLINS_OVERLAY_DIR") {
        if !ov.is_empty() {
            return PathBuf::from(ov);
        }
    }
    project_root.join(crate::config::OVERLAY_DIRNAME)
}

fn project_pipeline_dirs(project_root: &std::path::Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for d in &[
        project_overlay_dir(project_root),
        project_root.join(crate::config::OVERLAY_DIRNAME),
    ] {
        let resolved = d.canonicalize().unwrap_or_else(|_| d.clone());
        if seen.insert(resolved) {
            dirs.push(d.clone());
        }
    }
    dirs
}

pub fn list_pipelines(
    project_root: PathBuf,
    bundled_pipelines_dir: PathBuf,
) -> Vec<(String, PathBuf)> {
    let mut results: Vec<(String, PathBuf)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for local_dir in project_pipeline_dirs(&project_root) {
        if !local_dir.exists() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&local_dir) {
            let mut yamls: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "yaml"))
                .collect();
            yamls.sort();
            for p in yamls {
                let stem = p.file_stem().and_then(|s| s.to_str()).map(String::from);
                if let Some(stem) = stem {
                    if seen.insert(stem.clone()) {
                        let resolved = p.canonicalize().unwrap_or(p);
                        results.push((stem, resolved));
                    }
                }
            }
        }
    }

    let mut bundled_names: Vec<&str> = PIPELINES.keys().copied().collect();
    bundled_names.sort();
    for name in bundled_names {
        if seen.insert(name.to_string()) {
            let p = bundled_pipelines_dir.join(PIPELINE_PATHS[name]);
            results.push((name.to_string(), p.canonicalize().unwrap_or(p)));
        }
    }

    results
}

pub fn resolve_pipeline_name(
    name: &str,
    project_root: PathBuf,
    bundled_pipelines_dir: PathBuf,
) -> Result<PathBuf, DiscoveryError> {
    for d in project_pipeline_dirs(&project_root) {
        let candidate = d.join(format!("{}.yaml", name));
        if candidate.exists() {
            return Ok(candidate.canonicalize().unwrap_or(candidate));
        }
    }
    if PIPELINES.contains_key(name) {
        let p = bundled_pipelines_dir.join(PIPELINE_PATHS[name]);
        return Ok(p.canonicalize().unwrap_or(p));
    }

    let mut names: Vec<String> = Vec::new();
    for d in project_pipeline_dirs(&project_root) {
        if d.exists() {
            if let Ok(entries) = std::fs::read_dir(&d) {
                let mut stems: Vec<String> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|ext| ext == "yaml"))
                    .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(String::from))
                    .collect();
                stems.sort();
                names.extend(stems);
            }
        }
    }
    names.extend(PIPELINES.keys().map(|k| k.to_string()));
    let mut seen = HashSet::new();
    names.retain(|n| seen.insert(n.clone()));
    let available = names.join(", ");
    Err(DiscoveryError::Name {
        name: name.to_string(),
        available,
    })
}

pub fn resolve_pipeline_path(
    name_or_path: &str,
    base_dir: PathBuf,
    bundled_pipelines_dir: PathBuf,
) -> Result<PathBuf, DiscoveryError> {
    let candidate = PathBuf::from(name_or_path);
    if candidate.extension().is_some_and(|ext| ext == "yaml") || candidate.components().count() > 1
    {
        let resolved = candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.clone());
        if !resolved.exists() {
            return Err(DiscoveryError::File { path: resolved });
        }
        return Ok(resolved);
    }

    for d in project_pipeline_dirs(&base_dir) {
        let project_scoped = d.join(format!("{}.yaml", name_or_path));
        if project_scoped.exists() {
            return Ok(project_scoped.canonicalize().unwrap_or(project_scoped));
        }
    }
    if PIPELINES.contains_key(name_or_path) {
        let p = bundled_pipelines_dir.join(PIPELINE_PATHS[name_or_path]);
        return Ok(p.canonicalize().unwrap_or(p));
    }

    let dirs: Vec<String> = project_pipeline_dirs(&base_dir)
        .iter()
        .map(|d| d.display().to_string())
        .collect();
    Err(DiscoveryError::Path {
        name: name_or_path.to_string(),
        dirs: dirs.join(" or "),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_dirs() -> (TempDir, PathBuf) {
        std::env::remove_var("GREMLINS_OVERLAY_DIR");
        let project = TempDir::new().unwrap();
        let overlay = project.path().join(".gremlins");
        fs::create_dir_all(&overlay).unwrap();
        let bundled = TempDir::new().unwrap();
        // Write bundled pipeline files so tests can find them
        for (_name, fname) in &[
            ("boss", "boss.yaml"),
            ("gh", "gh.yaml"),
            ("gh-terse", "gh-terse.yaml"),
            ("local", "local.yaml"),
            ("pr-extend", "pr-extend.yaml"),
        ] {
            fs::write(bundled.path().join(fname), "stages: []").unwrap();
        }
        (project, bundled.path().to_path_buf())
    }

    #[test]
    fn test_list_pipelines_empty() {
        let (project, bundled) = setup_dirs();
        let result = list_pipelines(project.path().to_path_buf(), bundled);
        // bundled pipelines are always present
        assert!(!result.is_empty());
        assert!(result.iter().any(|(n, _)| n == "boss"));
    }

    #[test]
    fn test_list_pipelines_project_only() {
        let (project, bundled) = setup_dirs();
        let overlay = project.path().join(".gremlins");
        fs::write(overlay.join("foo.yaml"), "stages: []").unwrap();
        fs::write(overlay.join("bar.yaml"), "stages: []").unwrap();

        let result = list_pipelines(project.path().to_path_buf(), bundled);
        // project entries come first, then bundled
        assert!(result.len() >= 2);
        assert_eq!(result[0].0, "bar");
        assert_eq!(result[1].0, "foo");
    }

    #[test]
    fn test_list_pipelines_dedup() {
        let (project, bundled) = setup_dirs();
        let overlay = project.path().join(".gremlins");
        fs::create_dir_all(&overlay).unwrap();
        fs::write(overlay.join("dup.yaml"), "stages: []").unwrap();
        let result = list_pipelines(project.path().to_path_buf(), bundled);
        assert_eq!(result.iter().filter(|(n, _)| n == "dup").count(), 1);
    }

    #[test]
    fn test_list_pipelines_bundled_always_present() {
        let (project, bundled) = setup_dirs();
        let result = list_pipelines(project.path().to_path_buf(), bundled);
        assert!(result.iter().any(|(n, _)| n == "boss"));
    }

    #[test]
    fn test_list_pipelines_project_wins_over_bundled() {
        let (project, bundled) = setup_dirs();
        let overlay = project.path().join(".gremlins");
        fs::create_dir_all(&overlay).unwrap();
        fs::write(overlay.join("boss.yaml"), "stages: [a]").unwrap();

        let result = list_pipelines(project.path().to_path_buf(), bundled);
        // project "boss" should come before the bundled one
        let idx = result.iter().position(|(n, _)| n == "boss").unwrap();
        assert_eq!(result[idx].0, "boss");
        assert!(result[idx]
            .1
            .starts_with(overlay.canonicalize().unwrap_or(overlay).as_path()));
        assert_eq!(result.iter().filter(|(n, _)| n == "boss").count(), 1);
    }

    #[test]
    fn test_resolve_pipeline_name_found_overlay() {
        let (project, bundled) = setup_dirs();
        let overlay = project.path().join(".gremlins");
        fs::create_dir_all(&overlay).unwrap();
        fs::write(overlay.join("test.yaml"), "stages: []").unwrap();

        let result = resolve_pipeline_name("test", project.path().to_path_buf(), bundled).unwrap();
        assert!(result.ends_with("test.yaml"));
    }

    #[test]
    fn test_resolve_pipeline_name_found_bundled() {
        let (project, bundled) = setup_dirs();
        let result = resolve_pipeline_name("boss", project.path().to_path_buf(), bundled).unwrap();
        assert!(result.ends_with("boss.yaml"));
    }

    #[test]
    fn test_resolve_pipeline_name_not_found() {
        let (project, bundled) = setup_dirs();
        let err = resolve_pipeline_name("nope", project.path().to_path_buf(), bundled).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("nope"));
        assert!(msg.contains("not found"));
    }

    #[test]
    fn test_resolve_pipeline_path_yaml_extension() {
        let (project, bundled) = setup_dirs();
        let overlay = project.path().join(".gremlins");
        fs::create_dir_all(&overlay).unwrap();
        let p = overlay.join("explicit.yaml");
        fs::write(&p, "stages: []").unwrap();

        let result =
            resolve_pipeline_path(p.to_str().unwrap(), project.path().to_path_buf(), bundled)
                .unwrap();
        assert!(result.ends_with("explicit.yaml"));
    }

    #[test]
    fn test_resolve_pipeline_path_bare_name() {
        let (project, bundled) = setup_dirs();
        let overlay = project.path().join(".gremlins");
        fs::create_dir_all(&overlay).unwrap();
        fs::write(overlay.join("bare.yaml"), "stages: []").unwrap();

        let result = resolve_pipeline_path("bare", project.path().to_path_buf(), bundled).unwrap();
        assert!(result.ends_with("bare.yaml"));
    }

    #[test]
    fn test_resolve_pipeline_path_missing() {
        let (project, bundled) = setup_dirs();
        let err =
            resolve_pipeline_path("nope.yaml", project.path().to_path_buf(), bundled).unwrap_err();
        assert!(err.to_string().contains("not found"));
    }
}
