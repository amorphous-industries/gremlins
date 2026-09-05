use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::schemas::error::SchemaError;

pub const GREMLINS_PREFIX: &str = "gremlins:";

pub const STAGE_TYPES: &[(&str, &str, &str)] = &[
    ("agent", "gremlins.stages.agent", "Agent"),
    ("loop", "gremlins.stages.loop", "LoopStage"),
    ("parallel", "gremlins.stages.parallel", "ParallelStage"),
    ("sequence", "gremlins.stages.sequence", "SequenceStage"),
    ("exec", "gremlins.stages.exec", "Exec"),
];

fn lookup_stage_class(
    stage_type: &str,
    name: &str,
) -> Result<(&'static str, &'static str), SchemaError> {
    for &(st, module, class) in STAGE_TYPES {
        if st == stage_type {
            return Ok((module, class));
        }
    }
    Err(SchemaError::Stage {
        name: name.to_string(),
        msg: format!("unknown type {stage_type:?}"),
    })
}

pub fn parse_stage(py: Python<'_>, d: &Bound<'_, PyDict>, depth: usize) -> PyResult<Py<PyAny>> {
    if d.contains("parallel")? {
        let cls = py
            .import("gremlins.stages.parallel")?
            .getattr("ParallelStage")?;
        let stage: Py<PyAny> = cls.call_method1("with_dict", (d, depth))?.extract()?;
        let name: String = d
            .get_item("name")?
            .and_then(|v| v.extract().ok())
            .unwrap_or_default();
        let skip_if_exists = parse_skip_if_exists(d, &name)?;
        stage.setattr(py, "raw_dict", d)?;
        stage.setattr(py, "skip_if_exists", skip_if_exists)?;
        return Ok(stage);
    }

    let name: String = d
        .get_item("name")?
        .and_then(|v| v.extract().ok())
        .unwrap_or_default();

    if d.contains("max_concurrent")? {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "stage {name:?}: 'max_concurrent' is only valid on parallel groups"
        )));
    }

    let stage_type: Option<String> = d.get_item("type")?.and_then(|v| v.extract().ok());

    let stage_type = match stage_type {
        Some(t) if !t.is_empty() => t,
        _ => {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "stage {name:?}: must have a 'type' field"
            )));
        }
    };

    // First try the built-in Rust constant
    if let Ok((module, class)) = lookup_stage_class(&stage_type, &name) {
        let cls = py.import(module)?.getattr(class)?;
        let stage: Py<PyAny> = cls.call_method1("with_dict", (d, depth))?.extract()?;
        let skip_if_exists = parse_skip_if_exists(d, &name)?;
        stage.setattr(py, "raw_dict", d)?;
        stage.setattr(py, "skip_if_exists", skip_if_exists)?;
        return Ok(stage);
    }

    // Fall back to the Python STAGE_TYPES dict (which may have dynamically
    // registered types, e.g. test fixtures).
    let stage_types: Bound<'_, PyDict> = py
        .import("_gremlins_core.schemas")?
        .getattr("STAGE_TYPES")?
        .cast_into()?;
    match stage_types.get_item(&stage_type)? {
        Some(cls) => {
            let stage: Py<PyAny> = cls.call_method1("with_dict", (d, depth))?.extract()?;
            let skip_if_exists = parse_skip_if_exists(d, &name)?;
            stage.setattr(py, "raw_dict", d)?;
            stage.setattr(py, "skip_if_exists", skip_if_exists)?;
            Ok(stage)
        }
        None => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "stage {name:?}: unknown type {stage_type:?}"
        ))),
    }
}

fn parse_skip_if_exists(d: &Bound<'_, PyDict>, name: &str) -> PyResult<String> {
    let raw = d.get_item("skip_if_exists")?;
    match raw {
        None => Ok(String::new()),
        Some(v) => {
            let s: String = v.extract().map_err(|_| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "stage {name:?}: 'skip_if_exists' must be a string, got {} type",
                    v.get_type()
                        .name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| "?".to_string())
                ))
            })?;
            if s.is_empty() {
                Ok(String::new())
            } else {
                Ok(s)
            }
        }
    }
}

pub fn parse_stages(
    py: Python<'_>,
    raw: &Bound<'_, PyList>,
    depth: usize,
) -> PyResult<Vec<Py<PyAny>>> {
    fill_names(raw)?;
    let mut stages = Vec::new();
    for item in raw.iter() {
        let d: &Bound<'_, PyDict> = item.cast()?;
        stages.push(parse_stage(py, d, depth)?);
    }
    Ok(stages)
}

pub fn fill_names(raw_stages: &Bound<'_, PyList>) -> PyResult<()> {
    let len = raw_stages.len();

    let mut explicit: Vec<String> = Vec::new();
    for i in 0..len {
        let item = raw_stages.get_item(i)?;
        let d: &Bound<'_, PyDict> = item.cast()?;
        if let Ok(Some(name)) = d
            .get_item("name")
            .map(|v| v.and_then(|v| v.extract::<String>().ok()))
        {
            if !name.is_empty() {
                explicit.push(name);
            }
        }
    }
    let mut used: std::collections::HashSet<String> = explicit.iter().cloned().collect();
    let mut counts: HashMap<String, usize> = HashMap::new();

    for i in 0..len {
        let item = raw_stages.get_item(i)?;
        let d: &Bound<'_, PyDict> = item.cast()?;

        let has_name = d
            .get_item("name")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok())
            .is_some_and(|n| !n.is_empty());

        if has_name {
            d.del_item("_auto_name").ok();
            continue;
        }

        let auto_raw: Option<String> = d.get_item("_auto_name").ok().flatten().and_then(|v| {
            v.str()
                .ok()
                .and_then(|s| s.to_str().ok().map(|s| s.to_string()))
        });
        d.del_item("_auto_name").ok();

        let auto = auto_raw.unwrap_or_default();
        let stage_type = if !auto.is_empty() {
            auto
        } else if d.contains("parallel").unwrap_or(false) {
            "parallel".to_string()
        } else {
            d.get_item("type")
                .ok()
                .flatten()
                .and_then(|v| v.extract::<String>().ok())
                .unwrap_or_default()
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
        d.set_item("name", &candidate)?;
        used.insert(candidate);
    }

    Ok(())
}

pub fn check_duplicate_producers(
    stages: &Bound<'_, PyList>,
    extra_out: Option<&Bound<'_, PyDict>>,
) -> PyResult<()> {
    _check_scope(stages, extra_out)
}

fn _check_scope(stages: &Bound<'_, PyList>, extra_out: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
    let py = stages.py();
    let len = stages.len();
    let mut seen: HashMap<String, (String, String)> = HashMap::new();

    if let Some(extra) = extra_out {
        for (raw_key, uri_str) in extra.iter() {
            let raw_key: String = raw_key.extract()?;
            let uri_str: String = uri_str.extract()?;
            let key = if raw_key.ends_with('?') {
                raw_key[..raw_key.len() - 1].to_string()
            } else {
                raw_key
            };
            seen.insert(key, ("bootstrap".to_string(), uri_str));
        }
    }

    for i in 0..len {
        let stage = stages.get_item(i)?;
        let bind_map_attr = stage.getattr("bind_map").ok();
        let bind_map: Option<&Bound<'_, PyDict>> =
            bind_map_attr.as_ref().and_then(|v| v.cast().ok());

        if let Some(bind_map) = bind_map {
            for (raw_key, uri_str) in bind_map.iter() {
                let raw_key: String = raw_key.extract()?;
                if raw_key.ends_with('?') {
                    continue;
                }
                let uri_str: String = uri_str.extract()?;
                let name: String = stage.getattr("name")?.extract()?;

                if let Some((prev_name, prev_uri)) = seen.get(&raw_key) {
                    if prev_uri != &uri_str {
                        let skip_if_exists: String = stage
                            .getattr("skip_if_exists")
                            .ok()
                            .and_then(|v| v.extract().ok())
                            .unwrap_or_default();
                        if skip_if_exists.is_empty() {
                            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                                "duplicate bind: key {raw_key:?}: declared by both {prev_name:?} and {name:?}"
                            )));
                        }
                    }
                } else {
                    seen.insert(raw_key, (name, uri_str));
                }
            }
        }

        let body_attr = stage.getattr("body").ok();
        let body: Option<&Bound<'_, PyList>> = body_attr.as_ref().and_then(|v| v.cast().ok());

        if let Some(body) = body {
            let is_parallel: bool = stage
                .getattr("type")
                .ok()
                .and_then(|v| v.extract::<String>().ok())
                .is_some_and(|t| t == "parallel");

            if is_parallel {
                for j in 0..body.len() {
                    let child = body.get_item(j)?;
                    let child_list = PyList::new(py, &[child])?;
                    _check_scope(&child_list, None)?;
                }
            } else {
                _check_scope(body, None)?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::types::{PyDict, PyList, PyTuple};

    #[test]
    fn test_fill_names_basic() {
        Python::attach(|py| {
            let items1 =
                PyList::new(py, vec![PyTuple::new(py, vec!["type", "agent"]).unwrap()]).unwrap();
            let dict1 = PyDict::from_sequence(&items1).unwrap();
            let items2 =
                PyList::new(py, vec![PyTuple::new(py, vec!["type", "agent"]).unwrap()]).unwrap();
            let dict2 = PyDict::from_sequence(&items2).unwrap();
            let list = PyList::new(py, [dict1, dict2]).unwrap();
            fill_names(&list).unwrap();
            let item0 = list.get_item(0).unwrap();
            let item1 = list.get_item(1).unwrap();
            let d1: &Bound<'_, PyDict> = item0.cast().unwrap();
            let d2: &Bound<'_, PyDict> = item1.cast().unwrap();
            assert_eq!(
                d1.get_item("name")
                    .unwrap()
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "agent"
            );
            assert_eq!(
                d2.get_item("name")
                    .unwrap()
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "agent-2"
            );
        });
    }

    #[test]
    fn test_fill_names_parallel() {
        Python::attach(|py| {
            let dict = PyDict::new(py);
            dict.set_item("parallel", PyList::empty(py)).unwrap();
            let list = PyList::new(py, [dict]).unwrap();
            fill_names(&list).unwrap();
            let item0 = list.get_item(0).unwrap();
            let d: &Bound<'_, PyDict> = item0.cast().unwrap();
            assert_eq!(
                d.get_item("name")
                    .unwrap()
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "parallel"
            );
        });
    }

    #[test]
    fn test_fill_names_explicit_wins() {
        Python::attach(|py| {
            let items = PyList::new(
                py,
                vec![
                    PyTuple::new(py, vec!["type", "agent"]).unwrap(),
                    PyTuple::new(py, vec!["name", "custom"]).unwrap(),
                ],
            )
            .unwrap();
            let dict = PyDict::from_sequence(&items).unwrap();
            let list = PyList::new(py, [dict]).unwrap();
            fill_names(&list).unwrap();
            let item0 = list.get_item(0).unwrap();
            let d: &Bound<'_, PyDict> = item0.cast().unwrap();
            assert_eq!(
                d.get_item("name")
                    .unwrap()
                    .unwrap()
                    .extract::<String>()
                    .unwrap(),
                "custom"
            );
        });
    }
}
