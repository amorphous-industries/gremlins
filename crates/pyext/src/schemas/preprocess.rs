use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyList};

use crate::schemas::error::SchemaError;
use crate::schemas::GREMLINS_PREFIX;

pub fn expand_pipeline(
    py: Python<'_>,
    yaml_path: PathBuf,
    project_root: Option<PathBuf>,
    bundled_stage_def_dir: PathBuf,
    bundled_prompt_dir: PathBuf,
    bundled_pipeline_dir: PathBuf,
    resolve_pipeline_name_fn: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let project_root = project_root.unwrap_or_else(|| {
        let parent = yaml_path.parent().unwrap_or(&yaml_path);
        if parent.file_name().is_some_and(|n| n == ".gremlins") {
            parent
                .parent()
                .map(PathBuf::from)
                .unwrap_or(PathBuf::from("."))
        } else {
            PathBuf::from(parent)
        }
    });

    let chain: Vec<PathBuf> = Vec::new();
    _expand(
        py,
        &yaml_path,
        &project_root,
        &chain,
        &bundled_stage_def_dir,
        &bundled_prompt_dir,
        &bundled_pipeline_dir,
        resolve_pipeline_name_fn,
    )
}

fn load_yaml_file(path: &PathBuf) -> Result<serde_yaml::Value, SchemaError> {
    let text = std::fs::read_to_string(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => SchemaError::PipelineFileNotFound {
            path: path.display().to_string(),
        },
        _ => SchemaError::Generic(format!("could not read {}: {}", path.display(), e)),
    })?;
    let parsed: serde_yaml::Value =
        serde_yaml::from_str(&text).map_err(|e| SchemaError::YamlParse {
            label: path.display().to_string(),
            msg: e.to_string(),
        })?;
    if !parsed.is_mapping() {
        return Err(SchemaError::YamlNotMapping {
            label: path.display().to_string(),
            got: format!("{:?}", parsed),
        });
    }
    Ok(parsed)
}

fn load_bundled_recipe(
    raw_name: &str,
    bundled_stage_def_dir: &PathBuf,
) -> Result<serde_yaml::Value, SchemaError> {
    let name = raw_name.replace('-', "_");
    let recipe_path = bundled_stage_def_dir.join(format!("{}.yaml", name));
    let recipe_path = recipe_path.canonicalize().unwrap_or(recipe_path);
    let bundled_dir = bundled_stage_def_dir
        .canonicalize()
        .unwrap_or_else(|_| bundled_stage_def_dir.clone());

    if !recipe_path.starts_with(&bundled_dir) {
        return Err(SchemaError::Generic(format!(
            "invalid bundled recipe name: {raw_name:?}"
        )));
    }
    if !recipe_path.exists() {
        let mut available = Vec::new();
        if let Ok(entries) = std::fs::read_dir(bundled_stage_def_dir) {
            for entry in entries.flatten() {
                if let Some(stem) = entry.path().file_stem().and_then(|s| s.to_str()) {
                    available.push(stem.to_string());
                }
            }
        }
        available.sort();
        return Err(SchemaError::BundledRecipeNotFound {
            name: format!("{GREMLINS_PREFIX}{raw_name}"),
            available: available.join(", "),
        });
    }
    load_yaml_file(&recipe_path)
}

#[allow(clippy::too_many_arguments)]
fn _expand(
    py: Python<'_>,
    yaml_path: &PathBuf,
    project_root: &PathBuf,
    chain: &[PathBuf],
    bundled_stage_def_dir: &PathBuf,
    bundled_prompt_dir: &PathBuf,
    bundled_pipeline_dir: &PathBuf,
    resolve_pipeline_name_fn: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let resolved = yaml_path
        .canonicalize()
        .unwrap_or_else(|_| yaml_path.clone());
    if chain.contains(&resolved) {
        let mut cycle_parts: Vec<String> = chain.iter().map(|p| p.display().to_string()).collect();
        cycle_parts.push(resolved.display().to_string());
        return Err(SchemaError::IncludeCycle(cycle_parts.join(" -> ")).into());
    }

    let raw = load_yaml_file(yaml_path)?;
    let raw_mapping = raw.as_mapping().unwrap();

    if raw_mapping
        .get("__gremlins_expanded__")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let mut result = raw.clone();
        if let Some(m) = result.as_mapping_mut() {
            m.remove("__gremlins_expanded__");
        }
        return serde_yaml_to_py(py, &result);
    }

    let yaml_dir = yaml_path.parent().unwrap_or(yaml_path);

    let prompt_dir = resolve_prompt_dir(raw_mapping.get("prompt_dir"), yaml_dir)?;

    let new_chain: Vec<PathBuf> = chain
        .iter()
        .chain(std::iter::once(&resolved))
        .cloned()
        .collect();

    let named_prompts =
        parse_named_prompts(raw_mapping.get("prompts"), &prompt_dir, bundled_prompt_dir)?;

    let stage_defs =
        parse_stage_definitions(raw_mapping.get("stage-definitions"), bundled_stage_def_dir)?;

    let stages_raw = raw_mapping.get("stages");
    let stages_list: Vec<serde_yaml::Value> = match stages_raw {
        None | Some(serde_yaml::Value::Null) => Vec::new(),
        Some(v) if v.is_sequence() => v.as_sequence().cloned().unwrap_or_default(),
        Some(_v) => {
            return Err(SchemaError::Generic("'stages' must be a list".to_string()).into());
        }
    };

    let mut expanded_stages: Vec<serde_yaml::Value> = Vec::new();
    for entry in stages_list {
        let expanded = _expand_entry(
            py,
            &entry,
            &prompt_dir,
            project_root,
            &new_chain,
            &named_prompts,
            &stage_defs,
            &HashSet::new(),
            bundled_stage_def_dir,
            bundled_prompt_dir,
            bundled_pipeline_dir,
            resolve_pipeline_name_fn,
        )?;
        expanded_stages.extend(expanded);
    }

    let mut result = serde_yaml::Mapping::new();
    for (k, v) in raw_mapping {
        let key_str = k.as_str().unwrap_or("");
        if key_str == "stages"
            || key_str == "prompt_dir"
            || key_str == "prompts"
            || key_str == "stage-definitions"
        {
            continue;
        }
        result.insert(k.clone(), v.clone());
    }
    result.insert(
        serde_yaml::Value::String("stages".to_string()),
        serde_yaml::Value::Sequence(expanded_stages),
    );

    serde_yaml_to_py(py, &serde_yaml::Value::Mapping(result))
}

fn resolve_prompt_dir(
    value: Option<&serde_yaml::Value>,
    yaml_dir: &std::path::Path,
) -> Result<PathBuf, SchemaError> {
    match value {
        None => Ok(PathBuf::from(yaml_dir)),
        Some(v) => {
            if let Some(s) = v.as_str() {
                let p = PathBuf::from(s);
                if p.is_absolute() {
                    Ok(p)
                } else {
                    Ok(yaml_dir.join(&p))
                }
            } else {
                Err(SchemaError::Generic(format!(
                    "prompt_dir must be a string, got {:?}",
                    v
                )))
            }
        }
    }
}

fn parse_named_prompts(
    prompts_raw: Option<&serde_yaml::Value>,
    prompt_dir: &std::path::Path,
    bundled_prompt_dir: &std::path::Path,
) -> Result<HashMap<String, Vec<String>>, SchemaError> {
    let mut named: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(mapping) = prompts_raw.and_then(|v| v.as_mapping()) {
        for (k, v) in mapping {
            let name = k.as_str().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            let empty: HashMap<String, Vec<String>> = HashMap::new();
            let texts = read_prompts(v, prompt_dir, &empty, bundled_prompt_dir)?;
            named.insert(name, texts);
        }
    }
    Ok(named)
}

fn parse_stage_definitions(
    raw: Option<&serde_yaml::Value>,
    bundled_stage_def_dir: &PathBuf,
) -> Result<HashMap<String, serde_yaml::Value>, SchemaError> {
    let mut defs: HashMap<String, serde_yaml::Value> = HashMap::new();
    match raw {
        None => {}
        Some(v) if v.is_mapping() => {
            let mapping = v.as_mapping().unwrap();
            for (k, v) in mapping {
                let name = k.as_str().unwrap_or("").to_string();
                if name.is_empty() {
                    continue;
                }
                if let Some(s) = v.as_str() {
                    if let Some(recipe_name) = s.strip_prefix(GREMLINS_PREFIX) {
                        if recipe_name.is_empty() {
                            return Err(SchemaError::StageDef {
                                name: name.clone(),
                                msg: format!("missing name after {GREMLINS_PREFIX:?}"),
                            });
                        }
                        match load_bundled_recipe(recipe_name, bundled_stage_def_dir) {
                            Ok(recipe) => {
                                defs.insert(name, recipe);
                            }
                            Err(e) => {
                                return Err(SchemaError::StageDef {
                                    name: name.clone(),
                                    msg: e.to_string(),
                                });
                            }
                        }
                    } else {
                        return Err(SchemaError::StageDef {
                            name: name.clone(),
                            msg: "must be a dict or gremlins: reference".to_string(),
                        });
                    }
                } else if v.is_mapping() {
                    defs.insert(name, v.clone());
                } else {
                    return Err(SchemaError::StageDef {
                        name: name.clone(),
                        msg: "must be a dict or gremlins: reference".to_string(),
                    });
                }
            }
        }
        Some(v) => {
            return Err(SchemaError::Generic(format!(
                "stage-definitions must be a mapping, got {:?}",
                v
            )));
        }
    }
    Ok(defs)
}

fn read_prompts(
    prompt_field: &serde_yaml::Value,
    prompt_dir: &std::path::Path,
    named_prompts: &HashMap<String, Vec<String>>,
    bundled_prompt_dir: &std::path::Path,
) -> Result<Vec<String>, SchemaError> {
    let raw: Vec<String> = match prompt_field {
        serde_yaml::Value::String(s) => vec![s.clone()],
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .map(|v| match v {
                serde_yaml::Value::String(s) => s.clone(),
                other => format!("{other:?}"),
            })
            .collect(),
        _ => {
            return Err(SchemaError::Generic(format!(
                "prompt must be a string or list, got {:?}",
                prompt_field
            )));
        }
    };

    let mut texts: Vec<String> = Vec::new();
    for p in &raw {
        if let Some(named) = named_prompts.get(p) {
            texts.extend(named.clone());
        } else if let Some(name) = p.strip_prefix(GREMLINS_PREFIX) {
            if name.is_empty() {
                return Err(SchemaError::Generic(format!(
                    "prompt {p:?} is missing a name after {GREMLINS_PREFIX:?}"
                )));
            }
            let path = bundled_prompt_dir.join(name);
            texts.push(read_prompt_file(&path)?);
        } else if p.contains('\n') {
            texts.push(p.clone());
        } else {
            let path = if PathBuf::from(p).is_absolute() {
                PathBuf::from(p)
            } else {
                prompt_dir.join(p)
            };
            let path = path.canonicalize().unwrap_or(path);
            if !path.exists() && !named_prompts.is_empty() {
                return Err(SchemaError::PromptFileNotFound {
                    path: format!(
                        "{p:?} not found as a named entry or file under {}",
                        prompt_dir.display()
                    ),
                });
            }
            texts.push(read_prompt_file(&path)?);
        }
    }

    Ok(texts)
}

fn read_prompt_file(path: &PathBuf) -> Result<String, SchemaError> {
    if !path.exists() {
        return Err(SchemaError::PromptFileNotFound {
            path: path.display().to_string(),
        });
    }
    let text = std::fs::read_to_string(path).map_err(|_| SchemaError::PromptFileNotFound {
        path: path.display().to_string(),
    })?;
    if text.trim().is_empty() {
        return Err(SchemaError::PromptFileEmpty {
            path: path.display().to_string(),
        });
    }
    Ok(text)
}

#[allow(clippy::too_many_arguments)]
fn _expand_entry(
    py: Python<'_>,
    entry: &serde_yaml::Value,
    prompt_dir: &PathBuf,
    project_root: &PathBuf,
    chain: &[PathBuf],
    named_prompts: &HashMap<String, Vec<String>>,
    stage_defs: &HashMap<String, serde_yaml::Value>,
    seen_defs: &HashSet<String>,
    bundled_stage_def_dir: &PathBuf,
    bundled_prompt_dir: &PathBuf,
    bundled_pipeline_dir: &PathBuf,
    resolve_pipeline_name_fn: &Bound<'_, PyAny>,
) -> PyResult<Vec<serde_yaml::Value>> {
    let mapping = match entry.as_mapping() {
        Some(m) => m,
        None => return Ok(vec![entry.clone()]),
    };

    // include: single-key entry
    if mapping.len() == 1 && mapping.contains_key("include") {
        let name = mapping
            .get("include")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if name.is_empty() {
            return Err(SchemaError::Generic(
                "include: value must be a non-empty string".to_string(),
            )
            .into());
        }
        let included_path: String = resolve_pipeline_name_fn
            .call1((name, project_root.clone(), bundled_pipeline_dir.clone()))?
            .extract()?;
        let included_path = PathBuf::from(included_path);
        let included = _expand(
            py,
            &included_path,
            project_root,
            chain,
            bundled_stage_def_dir,
            bundled_prompt_dir,
            bundled_pipeline_dir,
            resolve_pipeline_name_fn,
        )?;
        let included_dict: &Bound<'_, PyDict> = included.bind(py).cast()?;
        let stages: Vec<serde_yaml::Value> = included_dict
            .get_item("stages")?
            .map(|v| py_to_serde_yaml(&v))
            .transpose()?
            .map(|v| match v {
                serde_yaml::Value::Sequence(s) => s,
                _ => Vec::new(),
            })
            .unwrap_or_default();
        return Ok(stages);
    }

    let stage_type = mapping.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if !stage_type.is_empty() {
        if let Some(_def) = stage_defs.get(stage_type) {
            return _expand_stage_def(
                py,
                entry,
                stage_type,
                stage_defs,
                prompt_dir,
                project_root,
                chain,
                named_prompts,
                seen_defs,
                bundled_stage_def_dir,
                bundled_prompt_dir,
                bundled_pipeline_dir,
                resolve_pipeline_name_fn,
            );
        }
        if let Some(recipe_name) = stage_type.strip_prefix(GREMLINS_PREFIX) {
            if recipe_name.is_empty() {
                return Err(SchemaError::Generic(format!(
                    "missing name after {GREMLINS_PREFIX:?}"
                ))
                .into());
            }
            let recipe_def = load_bundled_recipe(recipe_name, bundled_stage_def_dir)?;
            let mut direct_defs = stage_defs.clone();
            direct_defs.insert(stage_type.to_string(), recipe_def);
            return _expand_stage_def(
                py,
                entry,
                stage_type,
                &direct_defs,
                prompt_dir,
                project_root,
                chain,
                named_prompts,
                seen_defs,
                bundled_stage_def_dir,
                bundled_prompt_dir,
                bundled_pipeline_dir,
                resolve_pipeline_name_fn,
            );
        }
        // Auto-resolve bundled stage-definitions by type name
        let recipe_path =
            bundled_stage_def_dir.join(format!("{}.yaml", stage_type.replace('-', "_")));
        if recipe_path.exists() {
            let auto_def = load_yaml_file(&recipe_path)?;
            let mut auto_defs = stage_defs.clone();
            auto_defs.insert(stage_type.to_string(), auto_def);
            return _expand_stage_def(
                py,
                entry,
                stage_type,
                &auto_defs,
                prompt_dir,
                project_root,
                chain,
                named_prompts,
                seen_defs,
                bundled_stage_def_dir,
                bundled_prompt_dir,
                bundled_pipeline_dir,
                resolve_pipeline_name_fn,
            );
        }
        // Try resolving as pipeline name
        let pipeline_result = resolve_pipeline_name_fn.call1((
            stage_type,
            project_root.clone(),
            bundled_pipeline_dir.clone(),
        ));
        match pipeline_result {
            Ok(result) => {
                let path: String = result.extract()?;
                let included_path = PathBuf::from(path);
                if !chain.contains(&included_path) {
                    let included = _expand(
                        py,
                        &included_path,
                        project_root,
                        chain,
                        bundled_stage_def_dir,
                        bundled_prompt_dir,
                        bundled_pipeline_dir,
                        resolve_pipeline_name_fn,
                    )?;
                    let included_dict: &Bound<'_, PyDict> = included.bind(py).cast()?;
                    let stages: Vec<serde_yaml::Value> = included_dict
                        .get_item("stages")?
                        .map(|v| py_to_serde_yaml(&v))
                        .transpose()?
                        .map(|v| match v {
                            serde_yaml::Value::Sequence(s) => s,
                            _ => Vec::new(),
                        })
                        .unwrap_or_default();
                    return Ok(stages);
                }
            }
            Err(err) if err.is_instance_of::<pyo3::exceptions::PyFileNotFoundError>(py) => {
                // Not a pipeline — fall through to loader validation
            }
            Err(err) => return Err(err),
        }
    }

    let mut entry = entry.clone();
    let entry_map = entry.as_mapping_mut().unwrap();

    if entry_map.contains_key("prompt") {
        let prompt_val = entry_map.get("prompt").unwrap().clone();
        let texts = read_prompts(&prompt_val, prompt_dir, named_prompts, bundled_prompt_dir)?;
        entry_map.insert(
            serde_yaml::Value::String("prompt".to_string()),
            serde_yaml::Value::Sequence(texts.into_iter().map(serde_yaml::Value::String).collect()),
        );
    }

    if let Some(parallel_val) = entry_map.get("parallel") {
        if let Some(parallel_list) = parallel_val.as_sequence() {
            let mut expanded_parallel: Vec<serde_yaml::Value> = Vec::new();
            for child in parallel_list {
                let child_dict = child.as_mapping();
                let include_name = child_dict
                    .filter(|m| m.len() == 1)
                    .and_then(|m| m.get("include"))
                    .and_then(|v| v.as_str())
                    .map(String::from);

                let expanded = _expand_entry(
                    py,
                    child,
                    prompt_dir,
                    project_root,
                    chain,
                    named_prompts,
                    stage_defs,
                    seen_defs,
                    bundled_stage_def_dir,
                    bundled_prompt_dir,
                    bundled_pipeline_dir,
                    resolve_pipeline_name_fn,
                )?;

                if expanded.is_empty() {
                    return Err(SchemaError::Generic(
                        "parallel child expanded to 0 stages via include; includes inside parallel groups must resolve to at least one stage".to_string()
                    ).into());
                }
                if expanded.len() == 1 {
                    expanded_parallel.push(expanded.into_iter().next().unwrap());
                } else {
                    let name = include_name
                        .unwrap_or_else(|| format!("sequence-{}", expanded_parallel.len()));
                    let mut seq = serde_yaml::Mapping::new();
                    seq.insert(
                        serde_yaml::Value::String("name".to_string()),
                        serde_yaml::Value::String(name),
                    );
                    seq.insert(
                        serde_yaml::Value::String("type".to_string()),
                        serde_yaml::Value::String("sequence".to_string()),
                    );
                    seq.insert(
                        serde_yaml::Value::String("body".to_string()),
                        serde_yaml::Value::Sequence(expanded),
                    );
                    expanded_parallel.push(serde_yaml::Value::Mapping(seq));
                }
            }
            entry_map.insert(
                serde_yaml::Value::String("parallel".to_string()),
                serde_yaml::Value::Sequence(expanded_parallel),
            );
        }
    }

    if let Some(body_val) = entry_map.get("body") {
        if let Some(body_list) = body_val.as_sequence() {
            let mut expanded_body: Vec<serde_yaml::Value> = Vec::new();
            for body_entry in body_list {
                let expanded = _expand_entry(
                    py,
                    body_entry,
                    prompt_dir,
                    project_root,
                    chain,
                    named_prompts,
                    stage_defs,
                    seen_defs,
                    bundled_stage_def_dir,
                    bundled_prompt_dir,
                    bundled_pipeline_dir,
                    resolve_pipeline_name_fn,
                )?;
                expanded_body.extend(expanded);
            }
            entry_map.insert(
                serde_yaml::Value::String("body".to_string()),
                serde_yaml::Value::Sequence(expanded_body),
            );
        }
    }

    Ok(vec![entry])
}

#[allow(clippy::too_many_arguments)]
fn _expand_stage_def(
    py: Python<'_>,
    call_site: &serde_yaml::Value,
    def_name: &str,
    stage_defs: &HashMap<String, serde_yaml::Value>,
    prompt_dir: &PathBuf,
    project_root: &PathBuf,
    chain: &[PathBuf],
    named_prompts: &HashMap<String, Vec<String>>,
    seen_defs: &HashSet<String>,
    bundled_stage_def_dir: &PathBuf,
    bundled_prompt_dir: &PathBuf,
    bundled_pipeline_dir: &PathBuf,
    resolve_pipeline_name_fn: &Bound<'_, PyAny>,
) -> PyResult<Vec<serde_yaml::Value>> {
    if seen_defs.contains(def_name) {
        return Err(SchemaError::Generic(format!("stage-definition cycle: {def_name:?}")).into());
    }

    let definition = stage_defs
        .get(def_name)
        .ok_or_else(|| SchemaError::Generic(format!("stage-definition {def_name:?} not found")))?;

    let mut new_seen = seen_defs.clone();
    new_seen.insert(def_name.to_string());

    let def_map = definition.as_mapping().ok_or_else(|| {
        SchemaError::Generic(format!("stage-definition {def_name:?} is not a mapping"))
    })?;

    let call_site_map = call_site.as_mapping().unwrap();

    if let Some(inner_list) = def_map.get("stages").and_then(|v| v.as_sequence()) {
        if inner_list.is_empty() {
            return Err(SchemaError::StageDef {
                name: def_name.to_string(),
                msg: "'stages' must be a non-empty list".to_string(),
            }
            .into());
        }
        if def_map.contains_key("out") {
            return Err(SchemaError::StageDef {
                name: def_name.to_string(),
                msg: "must not declare 'out:' keys; declare them at each call site instead"
                    .to_string(),
            }
            .into());
        }

        let last_idx = inner_list.len() - 1;
        let required_opts: Vec<String> = def_map
            .get("required-options")
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let cs_opts: HashMap<String, serde_yaml::Value> = call_site_map
            .get("options")
            .and_then(|v| v.as_mapping())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.as_str().unwrap_or("").to_string(), v.clone()))
                    .collect()
            })
            .unwrap_or_default();

        for opt in &required_opts {
            let val = cs_opts.get(opt);
            let is_empty = match val {
                None => true,
                Some(serde_yaml::Value::Sequence(s)) => s.is_empty(),
                Some(serde_yaml::Value::Null) => true,
                _ => false,
            };
            if is_empty {
                let stage_display = call_site_map
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(def_name);
                return Err(SchemaError::Stage {
                    name: stage_display.to_string(),
                    msg: format!("required option {opt:?} is missing or empty"),
                }
                .into());
            }
        }

        let cs_prompts: Vec<String> = if call_site_map.contains_key("prompt") {
            read_prompts(
                call_site_map.get("prompt").unwrap(),
                prompt_dir,
                named_prompts,
                bundled_prompt_dir,
            )?
        } else {
            Vec::new()
        };

        if def_map
            .get("required-prompt")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            && cs_prompts.is_empty()
        {
            let stage_display = call_site_map
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(def_name);
            return Err(SchemaError::Stage {
                name: stage_display.to_string(),
                msg: "required prompt is missing or empty".to_string(),
            }
            .into());
        }

        let mut ctx = serde_yaml::Mapping::new();
        ctx.insert(
            serde_yaml::Value::String("options".to_string()),
            serde_yaml::Value::Mapping(
                cs_opts
                    .into_iter()
                    .map(|(k, v)| (serde_yaml::Value::String(k), v))
                    .collect(),
            ),
        );
        ctx.insert(
            serde_yaml::Value::String("prompt".to_string()),
            serde_yaml::Value::Sequence(
                cs_prompts
                    .iter()
                    .map(|s| serde_yaml::Value::String(s.clone()))
                    .collect(),
            ),
        );

        let ctx_value = serde_yaml::Value::Mapping(ctx);

        let mut result: Vec<serde_yaml::Value> = Vec::new();
        for (i, raw_inner) in inner_list.iter().enumerate() {
            let substituted = substitute_recipe(raw_inner, &ctx_value)?;
            let mut inner = substituted.clone();
            if !inner.is_mapping() {
                return Err(SchemaError::StageDef {
                    name: def_name.to_string(),
                    msg: format!("inner stage {i} must be a mapping, got {:?}", inner),
                }
                .into());
            }
            let inner_map = inner.as_mapping_mut().unwrap();

            if i == 0 {
                if let Some(name) = call_site_map.get("name") {
                    inner_map.insert(serde_yaml::Value::String("name".to_string()), name.clone());
                } else if inner_map.contains_key("name") {
                    let existing_name = inner_map.remove("name").unwrap();
                    inner_map.insert(
                        serde_yaml::Value::String("_auto_name".to_string()),
                        existing_name,
                    );
                }
                if let Some(client) = call_site_map.get("client") {
                    inner_map.insert(
                        serde_yaml::Value::String("client".to_string()),
                        client.clone(),
                    );
                }
                if let Some(in_val) = call_site_map.get("in") {
                    let mut merged_in = inner_map
                        .get("in")
                        .and_then(|v| v.as_mapping())
                        .map(|m| {
                            m.iter()
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect::<serde_yaml::Mapping>()
                        })
                        .unwrap_or_default();
                    if let Some(cs_in) = in_val.as_mapping() {
                        for (k, v) in cs_in {
                            merged_in.insert(k.clone(), v.clone());
                        }
                    }
                    inner_map.insert(
                        serde_yaml::Value::String("in".to_string()),
                        serde_yaml::Value::Mapping(merged_in),
                    );
                }
            }
            if i == last_idx {
                if let Some(out_val) = call_site_map.get("out") {
                    if inner_map.contains_key("out") {
                        return Err(SchemaError::StageDef {
                            name: def_name.to_string(),
                            msg: format!(
                                "inner stage {i} declares 'out:'; call-site must not also declare 'out:'"
                            ),
                        }.into());
                    }
                    inner_map.insert(
                        serde_yaml::Value::String("out".to_string()),
                        out_val.clone(),
                    );
                }
            }

            let expanded = _expand_entry(
                py,
                &inner,
                prompt_dir,
                project_root,
                chain,
                named_prompts,
                stage_defs,
                &new_seen,
                bundled_stage_def_dir,
                bundled_prompt_dir,
                bundled_pipeline_dir,
                resolve_pipeline_name_fn,
            )?;
            result.extend(expanded);
        }
        return Ok(result);
    }

    // Single-primitive definition
    if def_map.contains_key("out") {
        return Err(SchemaError::StageDef {
            name: def_name.to_string(),
            msg: "must not declare 'out:' keys; declare them at each call site instead".to_string(),
        }
        .into());
    }

    let mut merged = definition.clone();
    let merged_map = merged.as_mapping_mut().unwrap();

    for key in &["name", "in", "out"] {
        if let Some(v) = call_site_map.get(*key) {
            merged_map.insert(serde_yaml::Value::String(key.to_string()), v.clone());
        }
    }
    if !call_site_map.contains_key("name") && merged_map.contains_key("name") {
        let existing_name = merged_map.remove("name").unwrap();
        merged_map.insert(
            serde_yaml::Value::String("_auto_name".to_string()),
            existing_name,
        );
    }

    _expand_entry(
        py,
        &merged,
        prompt_dir,
        project_root,
        chain,
        named_prompts,
        stage_defs,
        &new_seen,
        bundled_stage_def_dir,
        bundled_prompt_dir,
        bundled_pipeline_dir,
        resolve_pipeline_name_fn,
    )
}

fn substitute_recipe(
    node: &serde_yaml::Value,
    ctx: &serde_yaml::Value,
) -> Result<serde_yaml::Value, SchemaError> {
    match node {
        serde_yaml::Value::Mapping(m) => {
            let mut out = serde_yaml::Mapping::new();
            for (k, v) in m {
                out.insert(k.clone(), substitute_recipe(v, ctx)?);
            }
            Ok(serde_yaml::Value::Mapping(out))
        }
        serde_yaml::Value::Sequence(seq) => {
            let mut out: Vec<serde_yaml::Value> = Vec::new();
            for item in seq {
                if let serde_yaml::Value::String(s) = item {
                    if s.starts_with("{{") && s.ends_with("}}") && s.matches("{{").count() == 1 {
                        let key = s[2..s.len() - 2].trim();
                        match resolve_placeholder(key, ctx) {
                            Ok(resolved) => {
                                if let serde_yaml::Value::Sequence(resolved_seq) = resolved {
                                    out.extend(resolved_seq);
                                } else {
                                    out.push(resolved);
                                }
                                continue;
                            }
                            Err(e) => {
                                return Err(SchemaError::Generic(e));
                            }
                        }
                    }
                }
                out.push(substitute_recipe(item, ctx)?);
            }
            Ok(serde_yaml::Value::Sequence(out))
        }
        serde_yaml::Value::String(s) => {
            if s.starts_with("{{") && s.ends_with("}}") && s.matches("{{").count() == 1 {
                let key = s[2..s.len() - 2].trim();
                match resolve_placeholder(key, ctx) {
                    Ok(resolved) => Ok(resolved),
                    Err(e) => Err(SchemaError::Generic(e)),
                }
            } else {
                Ok(node.clone())
            }
        }
        _ => Ok(node.clone()),
    }
}

fn resolve_placeholder(key: &str, ctx: &serde_yaml::Value) -> Result<serde_yaml::Value, String> {
    let (dotted_key, has_default, default_val) = if let Some(idx) = key.find(" | default(") {
        let raw_default = &key[idx + " | default(".len()..];
        let raw_default = raw_default.strip_suffix(')').unwrap_or(raw_default);
        let default = parse_default(raw_default);
        (key[..idx].trim(), true, default)
    } else {
        (key.trim(), false, serde_yaml::Value::Null)
    };

    let parts: Vec<&str> = dotted_key.split('.').collect();
    let mut val = ctx;
    for part in &parts {
        match val.as_mapping().and_then(|m| m.get(*part)) {
            Some(v) => val = v,
            None => {
                if has_default {
                    return Ok(default_val);
                }
                return Err(format!(
                    "placeholder {{{{{dotted_key}}}}}: key {part:?} not found in context"
                ));
            }
        }
    }

    match val {
        serde_yaml::Value::Mapping(_) | serde_yaml::Value::Sequence(_) => Ok(val.clone()),
        serde_yaml::Value::String(s) => Ok(serde_yaml::Value::String(s.clone())),
        serde_yaml::Value::Number(n) => Ok(serde_yaml::Value::String(n.to_string())),
        serde_yaml::Value::Bool(b) => Ok(serde_yaml::Value::String(b.to_string())),
        serde_yaml::Value::Null => Ok(serde_yaml::Value::String("null".to_string())),
        other => Ok(serde_yaml::Value::String(format!("{other:?}"))),
    }
}

fn parse_default(raw: &str) -> serde_yaml::Value {
    let s = raw.trim();
    if s.len() >= 2 {
        let first = s.chars().next().unwrap();
        let last = s.chars().last().unwrap();
        if first == last && (first == '"' || first == '\'') {
            return serde_yaml::Value::String(s[1..s.len() - 1].to_string());
        }
    }
    serde_yaml::Value::String(s.to_string())
}

fn serde_yaml_to_py(py: Python<'_>, value: &serde_yaml::Value) -> PyResult<Py<PyAny>> {
    match value {
        serde_yaml::Value::Null => Ok(py.None()),
        serde_yaml::Value::Bool(b) => Ok(PyBool::new(py, *b).to_owned().into_any().unbind()),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.into_pyobject(py)?.into_any().unbind())
            } else if let Some(f) = n.as_f64() {
                Ok(f.into_pyobject(py)?.into_any().unbind())
            } else {
                Ok(py.None())
            }
        }
        serde_yaml::Value::String(s) => Ok(s.into_pyobject(py)?.into_any().unbind()),
        serde_yaml::Value::Sequence(seq) => {
            let list = PyList::empty(py);
            for item in seq {
                list.append(serde_yaml_to_py(py, item)?)?;
            }
            Ok(list.into())
        }
        serde_yaml::Value::Mapping(m) => {
            let dict = PyDict::new(py);
            for (k, v) in m {
                let key_str = match k {
                    serde_yaml::Value::String(s) => s.clone(),
                    other => format!("{other:?}"),
                };
                dict.set_item(key_str, serde_yaml_to_py(py, v)?)?;
            }
            Ok(dict.into())
        }
        serde_yaml::Value::Tagged(t) => serde_yaml_to_py(py, &t.value),
    }
}

fn py_to_serde_yaml(value: &Bound<'_, PyAny>) -> PyResult<serde_yaml::Value> {
    if value.is_none() {
        return Ok(serde_yaml::Value::Null);
    }
    if let Ok(b) = value.extract::<bool>() {
        return Ok(serde_yaml::Value::Bool(b));
    }
    if let Ok(i) = value.extract::<i64>() {
        return Ok(serde_yaml::Value::Number(i.into()));
    }
    if let Ok(f) = value.extract::<f64>() {
        // serde_yaml 0.9 Number doesn't have from_f64; use serde_json
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Ok(serde_yaml::Value::Number(serde_yaml::Number::from(
                n.as_f64().unwrap_or(f),
            )));
        }
    }
    if let Ok(s) = value.extract::<String>() {
        return Ok(serde_yaml::Value::String(s));
    }
    if let Ok(list) = value.cast::<PyList>() {
        let mut seq = Vec::new();
        for item in list.iter() {
            seq.push(py_to_serde_yaml(&item)?);
        }
        return Ok(serde_yaml::Value::Sequence(seq));
    }
    if let Ok(dict) = value.cast::<PyDict>() {
        let mut map = serde_yaml::Mapping::new();
        for (k, v) in dict.iter() {
            let key_str: String = k.extract()?;
            map.insert(serde_yaml::Value::String(key_str), py_to_serde_yaml(&v)?);
        }
        return Ok(serde_yaml::Value::Mapping(map));
    }
    let s: String = value.str()?.to_string_lossy().into_owned();
    Ok(serde_yaml::Value::String(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_substitute_recipe_simple() {
        let mut ctx_map = serde_yaml::Mapping::new();
        ctx_map.insert(
            serde_yaml::Value::String("options".to_string()),
            serde_yaml::Value::Mapping({
                let mut m = serde_yaml::Mapping::new();
                m.insert(
                    serde_yaml::Value::String("key".to_string()),
                    serde_yaml::Value::String("value".to_string()),
                );
                m
            }),
        );
        let ctx = serde_yaml::Value::Mapping(ctx_map);

        let input = serde_yaml::Value::String("{{options.key}}".to_string());
        let result = substitute_recipe(&input, &ctx).unwrap();
        assert_eq!(result.as_str().unwrap(), "value");
    }

    #[test]
    fn test_substitute_recipe_default() {
        let ctx = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        let input =
            serde_yaml::Value::String("{{options.missing | default(fallback)}}".to_string());
        let result = substitute_recipe(&input, &ctx).unwrap();
        assert_eq!(result.as_str().unwrap(), "fallback");
    }

    #[test]
    fn test_substitute_recipe_missing_placeholder_errors() {
        let ctx = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        let input = serde_yaml::Value::String("{{options.missing}}".to_string());
        let err = substitute_recipe(&input, &ctx).unwrap_err();
        assert!(err.to_string().contains("not found in context"));
    }

    #[test]
    fn test_parse_default_quoted() {
        let result = parse_default("\"hello\"");
        assert_eq!(result.as_str().unwrap(), "hello");
    }

    #[test]
    fn test_parse_default_unquoted() {
        let result = parse_default("hello");
        assert_eq!(result.as_str().unwrap(), "hello");
    }
}
