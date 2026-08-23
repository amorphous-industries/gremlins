use std::collections::HashSet;
use std::path::PathBuf;

mod error;
use error::DiscoveryError;

const OVERLAY_DIRNAME: &str = ".gremlins";

fn project_overlay_dir(project_root: &std::path::Path) -> PathBuf {
    if let Ok(ov) = std::env::var("GREMLINS_OVERLAY_DIR") {
        if !ov.is_empty() {
            return PathBuf::from(ov);
        }
    }
    project_root.join(OVERLAY_DIRNAME)
}

fn project_pipeline_dirs(project_root: &std::path::Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for d in &[
        project_overlay_dir(project_root),
        project_root.join(OVERLAY_DIRNAME),
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
    bundled_pipeline_dir: PathBuf,
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

    if bundled_pipeline_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&bundled_pipeline_dir) {
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

    results
}

pub fn resolve_pipeline_name(
    name: &str,
    project_root: PathBuf,
    bundled_pipeline_dir: PathBuf,
) -> Result<PathBuf, DiscoveryError> {
    for d in project_pipeline_dirs(&project_root) {
        let candidate = d.join(format!("{}.yaml", name));
        if candidate.exists() {
            return Ok(candidate.canonicalize().unwrap_or(candidate));
        }
    }
    let bundled = bundled_pipeline_dir.join(format!("{}.yaml", name));
    if bundled.exists() {
        return Ok(bundled.canonicalize().unwrap_or(bundled));
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
    if bundled_pipeline_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&bundled_pipeline_dir) {
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
    bundled_pipeline_dir: PathBuf,
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
    let bundled = bundled_pipeline_dir.join(format!("{}.yaml", name_or_path));
    if bundled.exists() {
        return Ok(bundled.canonicalize().unwrap_or(bundled));
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

    fn setup_dirs() -> (TempDir, TempDir, TempDir) {
        std::env::remove_var("GREMLINS_OVERLAY_DIR");
        let project = TempDir::new().unwrap();
        let overlay = project.path().join(".gremlins");
        fs::create_dir_all(&overlay).unwrap();
        let bundled = TempDir::new().unwrap();
        (project, TempDir::new().unwrap(), bundled)
    }

    #[test]
    fn test_list_pipelines_empty() {
        let (project, _tmp, bundled) = setup_dirs();
        let result = list_pipelines(project.path().to_path_buf(), bundled.path().to_path_buf());
        assert!(result.is_empty());
    }

    #[test]
    fn test_list_pipelines_project_only() {
        let (project, _tmp, bundled) = setup_dirs();
        let overlay = project.path().join(".gremlins");
        fs::write(overlay.join("foo.yaml"), "stages: []").unwrap();
        fs::write(overlay.join("bar.yaml"), "stages: []").unwrap();

        let result = list_pipelines(project.path().to_path_buf(), bundled.path().to_path_buf());
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "bar");
        assert_eq!(result[1].0, "foo");
    }

    #[test]
    fn test_list_pipelines_dedup() {
        let (project, _tmp, bundled) = setup_dirs();
        let overlay = project.path().join(".gremlins");
        let _project_dir = project.path().join(".gremlins");
        fs::create_dir_all(&overlay).unwrap();
        fs::write(overlay.join("dup.yaml"), "stages: []").unwrap();
        // overlay and project_dir resolve to same path, so only one entry
        let result = list_pipelines(project.path().to_path_buf(), bundled.path().to_path_buf());
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_list_pipelines_bundled_fallback() {
        let (project, _tmp, bundled) = setup_dirs();
        fs::write(bundled.path().join("bundled.yaml"), "stages: []").unwrap();

        let result = list_pipelines(project.path().to_path_buf(), bundled.path().to_path_buf());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "bundled");
    }

    #[test]
    fn test_list_pipelines_project_wins_over_bundled() {
        let (project, _tmp, bundled) = setup_dirs();
        let overlay = project.path().join(".gremlins");
        fs::create_dir_all(&overlay).unwrap();
        fs::write(overlay.join("same.yaml"), "stages: [a]").unwrap();
        fs::write(bundled.path().join("same.yaml"), "stages: [b]").unwrap();

        let result = list_pipelines(project.path().to_path_buf(), bundled.path().to_path_buf());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "same");
    }

    #[test]
    fn test_resolve_pipeline_name_found_overlay() {
        let (project, _tmp, bundled) = setup_dirs();
        let overlay = project.path().join(".gremlins");
        fs::create_dir_all(&overlay).unwrap();
        fs::write(overlay.join("test.yaml"), "stages: []").unwrap();

        let result = resolve_pipeline_name(
            "test",
            project.path().to_path_buf(),
            bundled.path().to_path_buf(),
        )
        .unwrap();
        assert!(result.ends_with("test.yaml"));
    }

    #[test]
    fn test_resolve_pipeline_name_found_bundled() {
        let (project, _tmp, bundled) = setup_dirs();
        fs::write(bundled.path().join("test.yaml"), "stages: []").unwrap();

        let result = resolve_pipeline_name(
            "test",
            project.path().to_path_buf(),
            bundled.path().to_path_buf(),
        )
        .unwrap();
        assert!(result.ends_with("test.yaml"));
    }

    #[test]
    fn test_resolve_pipeline_name_not_found() {
        let (project, _tmp, bundled) = setup_dirs();
        let err = resolve_pipeline_name(
            "nope",
            project.path().to_path_buf(),
            bundled.path().to_path_buf(),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("nope"));
        assert!(msg.contains("not found"));
    }

    #[test]
    fn test_resolve_pipeline_path_yaml_extension() {
        let (project, _tmp, bundled) = setup_dirs();
        let overlay = project.path().join(".gremlins");
        fs::create_dir_all(&overlay).unwrap();
        let p = overlay.join("explicit.yaml");
        fs::write(&p, "stages: []").unwrap();

        let result = resolve_pipeline_path(
            p.to_str().unwrap(),
            project.path().to_path_buf(),
            bundled.path().to_path_buf(),
        )
        .unwrap();
        assert!(result.ends_with("explicit.yaml"));
    }

    #[test]
    fn test_resolve_pipeline_path_bare_name() {
        let (project, _tmp, bundled) = setup_dirs();
        let overlay = project.path().join(".gremlins");
        fs::create_dir_all(&overlay).unwrap();
        fs::write(overlay.join("bare.yaml"), "stages: []").unwrap();

        let result = resolve_pipeline_path(
            "bare",
            project.path().to_path_buf(),
            bundled.path().to_path_buf(),
        )
        .unwrap();
        assert!(result.ends_with("bare.yaml"));
    }

    #[test]
    fn test_resolve_pipeline_path_missing() {
        let (project, _tmp, bundled) = setup_dirs();
        let err = resolve_pipeline_path(
            "nope.yaml",
            project.path().to_path_buf(),
            bundled.path().to_path_buf(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }
}
