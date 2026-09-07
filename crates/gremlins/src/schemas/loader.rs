use std::collections::{HashMap, HashSet};

use crate::schemas::error::SchemaError;

/// A flattened stage descriptor used for name-filling.
pub struct StageEntry {
    pub name: Option<String>,
    pub auto_name: Option<String>,
    pub stage_type: Option<String>,
    pub is_parallel: bool,
}

/// A stage node for duplicate-producer checking.
pub struct StageNode {
    pub name: String,
    pub stage_type: String,
    pub bind_map: HashMap<String, String>,
    pub skip_if_exists: String,
    pub body: Vec<StageNode>,
}

pub fn fill_names(stages: &mut [StageEntry]) -> Result<(), SchemaError> {
    let mut used: HashSet<String> = HashSet::new();
    let mut name_counts: HashMap<String, usize> = HashMap::new();
    for stage in stages.iter() {
        if let Some(ref name) = stage.name {
            if !name.is_empty() {
                let count = name_counts.entry(name.clone()).or_insert(0);
                *count += 1;
            }
        }
    }

    let mut counts: HashMap<String, usize> = HashMap::new();

    for stage in stages.iter_mut() {
        if let Some(ref name) = stage.name {
            if !name.is_empty() {
                // If this explicit name is a duplicate, rename subsequent occurrences
                let count = name_counts.get(name.as_str()).copied().unwrap_or(0);
                if count > 1 && used.contains(name.as_str()) {
                    // This is a duplicate — append -N suffix
                    let base = name.clone();
                    let mut n = 2;
                    let mut candidate = format!("{base}-{n}");
                    while used.contains(&candidate) {
                        n += 1;
                        candidate = format!("{base}-{n}");
                    }
                    stage.name = Some(candidate.clone());
                    used.insert(candidate);
                } else {
                    used.insert(name.clone());
                }
                stage.auto_name = None;
                continue;
            }
        }

        let auto = stage.auto_name.take().unwrap_or_default();
        let stage_type = if !auto.is_empty() {
            auto
        } else if stage.is_parallel {
            "parallel".to_string()
        } else {
            stage.stage_type.clone().unwrap_or_default()
        };

        let count = counts.entry(stage_type.clone()).or_insert(0);
        *count += 1;
        let n = *count;
        let mut candidate = if n == 1 {
            stage_type.clone()
        } else {
            format!("{stage_type}-{n}")
        };

        while used.contains(&candidate) {
            *count += 1;
            let m = *count;
            candidate = format!("{stage_type}-{m}");
        }
        stage.name = Some(candidate.clone());
        used.insert(candidate);
    }

    Ok(())
}

pub fn check_duplicate_producers(
    stages: &[StageNode],
    extra_out: &HashMap<String, String>,
) -> Result<(), SchemaError> {
    let mut seen: HashMap<String, (String, String)> = HashMap::new();

    for (key, uri_str) in extra_out {
        if key.ends_with('?') {
            continue;
        }
        if !uri_str.starts_with("artifact://") {
            continue;
        }
        let clean_uri = uri_str.clone();
        if let Some((prev_name, _)) = seen.get(&clean_uri) {
            return Err(SchemaError::Generic(format!(
                "duplicate artifact producer: stage '{}' and stage '{}' both produce '{}'",
                prev_name, "bootstrap", clean_uri
            )));
        }
        seen.insert(clean_uri, ("bootstrap".to_string(), uri_str.clone()));
    }

    for stage in stages {
        // Stages with skip_if_exists set are conditional producers — they only
        // produce the artifact if it doesn't already exist, so they should not
        // trigger duplicate-producer errors.
        let is_conditional = !stage.skip_if_exists.is_empty();

        if !is_conditional {
            for (raw_key, uri_str) in &stage.bind_map {
                if raw_key.ends_with('?') {
                    continue;
                }
                if !uri_str.starts_with("artifact://") {
                    continue;
                }
                let clean_uri = uri_str.clone();
                if let Some((prev_name, _)) = seen.get(&clean_uri) {
                    return Err(SchemaError::Generic(format!(
                        "duplicate artifact producer: stage '{}' and stage '{}' both produce '{}'",
                        prev_name, stage.name, clean_uri
                    )));
                }
                seen.insert(clean_uri, (stage.name.clone(), uri_str.clone()));
            }
        }

        // For both parallel and sequential bodies, check each child in its own
        // scope.  Sequential stages intentionally overwrite artifacts (e.g. a
        // sanitize step rewrites rolling-plan.md), so siblings must not share
        // a seen map.
        for child in &stage.body {
            let child_slice = std::slice::from_ref(child);
            check_duplicate_producers(child_slice, &HashMap::new())?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fill_names_basic() {
        let mut stages = vec![
            StageEntry {
                name: None,
                auto_name: None,
                stage_type: Some("agent".to_string()),
                is_parallel: false,
            },
            StageEntry {
                name: None,
                auto_name: None,
                stage_type: Some("agent".to_string()),
                is_parallel: false,
            },
        ];
        fill_names(&mut stages).unwrap();
        assert_eq!(stages[0].name.as_deref(), Some("agent"));
        assert_eq!(stages[1].name.as_deref(), Some("agent-2"));
    }

    #[test]
    fn test_fill_names_parallel() {
        let mut stages = vec![StageEntry {
            name: None,
            auto_name: None,
            stage_type: None,
            is_parallel: true,
        }];
        fill_names(&mut stages).unwrap();
        assert_eq!(stages[0].name.as_deref(), Some("parallel"));
    }

    #[test]
    fn test_fill_names_explicit_wins() {
        let mut stages = vec![StageEntry {
            name: Some("custom".to_string()),
            auto_name: None,
            stage_type: Some("agent".to_string()),
            is_parallel: false,
        }];
        fill_names(&mut stages).unwrap();
        assert_eq!(stages[0].name.as_deref(), Some("custom"));
    }

    // --- check_duplicate_producers tests ---

    fn stage_with_bind(
        name: &str,
        stage_type: &str,
        bind_map: HashMap<String, String>,
    ) -> StageNode {
        StageNode {
            name: name.to_string(),
            stage_type: stage_type.to_string(),
            bind_map,
            skip_if_exists: String::new(),
            body: vec![],
        }
    }

    #[test]
    fn test_check_duplicate_producers_errs_on_different_uri() {
        let stages = vec![
            stage_with_bind(
                "s1",
                "agent",
                HashMap::from([("out".to_string(), "uri-a".to_string())]),
            ),
            stage_with_bind(
                "s2",
                "agent",
                HashMap::from([("result".to_string(), "uri-a".to_string())]),
            ),
        ];
        // Same URI value "uri-a" — no error (different keys, same value)
        check_duplicate_producers(&stages, &HashMap::new()).unwrap();
    }

    #[test]
    fn test_check_duplicate_producers_errs_on_same_uri_different_stages() {
        let stages = vec![
            stage_with_bind(
                "s1",
                "agent",
                HashMap::from([("out".to_string(), "artifact://plan.md".to_string())]),
            ),
            stage_with_bind(
                "s2",
                "agent",
                HashMap::from([("result".to_string(), "artifact://plan.md".to_string())]),
            ),
        ];
        let err = check_duplicate_producers(&stages, &HashMap::new()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("duplicate artifact producer"),
            "expected duplicate artifact producer error, got: {msg}"
        );
        assert!(
            msg.contains("artifact://plan.md"),
            "expected URI in error, got: {msg}"
        );
    }

    #[test]
    fn test_check_duplicate_producers_ok_on_different_uri() {
        let stages = vec![
            stage_with_bind(
                "s1",
                "agent",
                HashMap::from([("out".to_string(), "artifact://plan.md".to_string())]),
            ),
            stage_with_bind(
                "s2",
                "agent",
                HashMap::from([("out".to_string(), "artifact://spec.md".to_string())]),
            ),
        ];
        check_duplicate_producers(&stages, &HashMap::new()).unwrap();
    }

    #[test]
    fn test_check_duplicate_producers_skip_if_exists_bypasses_check() {
        let stages = vec![
            stage_with_bind(
                "s1",
                "agent",
                HashMap::from([("out".to_string(), "uri-a".to_string())]),
            ),
            StageNode {
                name: "s2".to_string(),
                stage_type: "agent".to_string(),
                bind_map: HashMap::from([("out".to_string(), "uri-b".to_string())]),
                skip_if_exists: "true".to_string(),
                body: vec![],
            },
        ];
        check_duplicate_producers(&stages, &HashMap::new()).unwrap();
    }

    #[test]
    fn test_parallel_children_isolated_scopes() {
        // Two children of a parallel stage can both produce the same URI
        // without triggering a duplicate error (isolated scopes).
        let stages = vec![StageNode {
            name: "par".to_string(),
            stage_type: "parallel".to_string(),
            bind_map: HashMap::new(),
            skip_if_exists: String::new(),
            body: vec![
                stage_with_bind(
                    "c1",
                    "agent",
                    HashMap::from([("out".to_string(), "artifact://plan.md".to_string())]),
                ),
                stage_with_bind(
                    "c2",
                    "agent",
                    HashMap::from([("out".to_string(), "artifact://plan.md".to_string())]),
                ),
            ],
        }];
        check_duplicate_producers(&stages, &HashMap::new()).unwrap();
    }

    #[test]
    fn test_non_parallel_children_shared_scope() {
        // Two children of a non-parallel (sequence) stage producing the same
        // URI is allowed — sequential stages intentionally overwrite artifacts
        // (e.g. a sanitize step rewrites rolling-plan.md).
        let stages = vec![StageNode {
            name: "seq".to_string(),
            stage_type: "sequence".to_string(),
            bind_map: HashMap::new(),
            skip_if_exists: String::new(),
            body: vec![
                stage_with_bind(
                    "c1",
                    "agent",
                    HashMap::from([("out".to_string(), "artifact://plan.md".to_string())]),
                ),
                stage_with_bind(
                    "c2",
                    "agent",
                    HashMap::from([("result".to_string(), "artifact://plan.md".to_string())]),
                ),
            ],
        }];
        check_duplicate_producers(&stages, &HashMap::new()).unwrap();
    }

    #[test]
    fn test_extra_out_collision_with_stage() {
        let stages = vec![stage_with_bind(
            "s1",
            "agent",
            HashMap::from([("out".to_string(), "artifact://plan.md".to_string())]),
        )];
        let extra_out = HashMap::from([("out".to_string(), "artifact://plan.md".to_string())]);
        let err = check_duplicate_producers(&stages, &extra_out).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("duplicate artifact producer"),
            "expected duplicate artifact producer error, got: {msg}"
        );
        assert!(
            msg.contains("bootstrap"),
            "expected 'bootstrap' in error, got: {msg}"
        );
    }

    #[test]
    fn test_extra_out_no_collision_on_different_uri() {
        let stages = vec![stage_with_bind(
            "s1",
            "agent",
            HashMap::from([("out".to_string(), "artifact://plan.md".to_string())]),
        )];
        let extra_out = HashMap::from([("out".to_string(), "artifact://spec.md".to_string())]);
        check_duplicate_producers(&stages, &extra_out).unwrap();
    }

    #[test]
    fn test_optional_bind_ignored_for_duplicates() {
        // Keys ending with '?' are optional — they should not trigger duplicates.
        let stages = vec![
            stage_with_bind(
                "s1",
                "agent",
                HashMap::from([("out".to_string(), "artifact://plan.md".to_string())]),
            ),
            stage_with_bind(
                "s2",
                "agent",
                HashMap::from([("out?".to_string(), "artifact://plan.md".to_string())]),
            ),
        ];
        check_duplicate_producers(&stages, &HashMap::new()).unwrap();
    }

    #[test]
    fn test_empty_stages_ok() {
        check_duplicate_producers(&[], &HashMap::new()).unwrap();
    }
}
