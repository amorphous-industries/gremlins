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
    let explicit: Vec<String> = stages
        .iter()
        .filter_map(|s| s.name.as_ref().filter(|n| !n.is_empty()).cloned())
        .collect();
    let mut used: HashSet<String> = explicit.iter().cloned().collect();
    let mut counts: HashMap<String, usize> = HashMap::new();

    for stage in stages.iter_mut() {
        if let Some(ref name) = stage.name {
            if !name.is_empty() {
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
        let clean_key = if key.ends_with('?') {
            key[..key.len() - 1].to_string()
        } else {
            key.clone()
        };
        seen.insert(clean_key, ("bootstrap".to_string(), uri_str.clone()));
    }

    for stage in stages {
        for (raw_key, uri_str) in &stage.bind_map {
            if raw_key.ends_with('?') {
                continue;
            }
            if let Some((prev_name, prev_uri)) = seen.get(raw_key) {
                if prev_uri != uri_str && stage.skip_if_exists.is_empty() {
                    return Err(SchemaError::Generic(format!(
                        "duplicate bind: key {raw_key:?}: declared by both {prev_name:?} and {name:?}",
                        name = stage.name
                    )));
                }
            } else {
                seen.insert(raw_key.clone(), (stage.name.clone(), uri_str.clone()));
            }
        }

        let is_parallel = stage.stage_type == "parallel";
        if is_parallel {
            for child in &stage.body {
                let child_slice = std::slice::from_ref(child);
                check_duplicate_producers(child_slice, &HashMap::new())?;
            }
        } else {
            check_duplicate_producers(&stage.body, &HashMap::new())?;
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
}