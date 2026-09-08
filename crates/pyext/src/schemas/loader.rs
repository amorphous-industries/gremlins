use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::schemas::error::SchemaError;
use gremlins::schemas::loader as core_loader;

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
            .unwrap_or_else(|| "<parallel>".to_string());
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
    let mut entries: Vec<core_loader::StageEntry> = Vec::new();
    for item in raw_stages.iter() {
        let d: &Bound<'_, PyDict> = item.cast()?;
        let name: Option<String> = d
            .get_item("name")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok())
            .filter(|n| !n.is_empty());
        let auto_name: Option<String> = d
            .get_item("_auto_name")
            .ok()
            .flatten()
            .and_then(|v| v.str().ok())
            .and_then(|s| s.to_str().ok().map(|s| s.to_string()))
            .filter(|n| !n.is_empty());
        d.del_item("_auto_name").ok();
        let stage_type: Option<String> = d
            .get_item("type")
            .ok()
            .flatten()
            .and_then(|v| v.extract::<String>().ok())
            .filter(|t| !t.is_empty());
        let is_parallel = d.contains("parallel").unwrap_or(false);
        entries.push(core_loader::StageEntry {
            name,
            auto_name,
            stage_type,
            is_parallel,
        });
    }

    core_loader::fill_names(&mut entries).map_err(|e: gremlins::schemas::error::SchemaError| {
        pyo3::exceptions::PyValueError::new_err(e.to_string())
    })?;

    for (i, entry) in entries.iter().enumerate() {
        if let Some(ref name) = entry.name {
            let item = raw_stages.get_item(i)?;
            let d: &Bound<'_, PyDict> = item.cast()?;
            d.set_item("name", name)?;
        }
    }

    Ok(())
}

pub fn check_duplicate_producers(
    stages: &Bound<'_, PyList>,
    extra_out: Option<&Bound<'_, PyDict>>,
) -> PyResult<()> {
    let nodes = py_stages_to_nodes(stages)?;
    let extra = match extra_out {
        Some(dict) => {
            let mut m = HashMap::new();
            for (k, v) in dict.iter() {
                let key: String = k.extract()?;
                let val: String = v.extract()?;
                m.insert(key, val);
            }
            m
        }
        None => HashMap::new(),
    };
    core_loader::check_duplicate_producers(&nodes, &extra).map_err(
        |e: gremlins::schemas::error::SchemaError| {
            pyo3::exceptions::PyValueError::new_err(e.to_string())
        },
    )
}

pub fn check_unresolved_consumers(
    stages: &Bound<'_, PyList>,
    launch_cmds: &Bound<'_, PyList>,
    cli_out: Option<&Bound<'_, PyDict>>,
) -> PyResult<()> {
    let nodes = py_stages_to_nodes(stages)?;
    let launch_cmds_vec: Vec<String> = launch_cmds
        .iter()
        .filter_map(|v| v.extract::<String>().ok())
        .collect();
    let cli_out_map = match cli_out {
        Some(dict) => {
            let mut m = HashMap::new();
            for (k, v) in dict.iter() {
                let key: String = k.extract()?;
                let val: String = v.extract()?;
                m.insert(key, val);
            }
            m
        }
        None => HashMap::new(),
    };
    core_loader::check_unresolved_consumers(&nodes, &launch_cmds_vec, &cli_out_map).map_err(
        |e: gremlins::schemas::error::SchemaError| {
            pyo3::exceptions::PyValueError::new_err(e.to_string())
        },
    )
}

pub fn py_stages_to_nodes(stages: &Bound<'_, PyList>) -> PyResult<Vec<core_loader::StageNode>> {
    let mut nodes = Vec::new();
    for item in stages.iter() {
        let name: String = item.getattr("name")?.extract()?;
        let stage_type: String = item
            .getattr("type")
            .ok()
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();
        let skip_if_exists: String = item
            .getattr("skip_if_exists")
            .ok()
            .and_then(|v| v.extract::<String>().ok())
            .unwrap_or_default();

        let mut bind_map = HashMap::new();
        if let Ok(bm) = item.getattr("bind_map") {
            if let Ok(bm_dict) = bm.cast::<PyDict>() {
                for (k, v) in bm_dict.iter() {
                    let key: String = k.extract()?;
                    let val: String = v.extract()?;
                    bind_map.insert(key, val);
                }
            }
        }

        let mut interpolation_map = HashMap::new();
        if let Ok(im) = item.getattr("interpolation_map") {
            if let Ok(im_dict) = im.cast::<PyDict>() {
                for (k, v) in im_dict.iter() {
                    let key: String = k.extract()?;
                    let val: String = v.extract()?;
                    interpolation_map.insert(key, val);
                }
            }
        }

        let mut body = Vec::new();
        if let Ok(body_attr) = item.getattr("body") {
            if let Ok(body_list) = body_attr.cast::<PyList>() {
                body = py_stages_to_nodes(body_list)?;
            }
        }

        nodes.push(core_loader::StageNode {
            name,
            stage_type,
            bind_map,
            interpolation_map,
            skip_if_exists,
            body,
        });
    }
    Ok(nodes)
}

pub fn assign_namespace_paths(py: Python<'_>, stages: &[Py<PyAny>]) -> PyResult<()> {
    let mut nodes = py_stages_to_namespace_nodes(py, stages)?;
    core_loader::assign_namespace_paths(&mut nodes)
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
    apply_namespace_paths(py, stages, &nodes)
}

fn py_stages_to_namespace_nodes(
    py: Python<'_>,
    stages: &[Py<PyAny>],
) -> PyResult<Vec<core_loader::NamespaceNode>> {
    let mut nodes = Vec::new();
    for stage in stages {
        let stage_ref = stage.bind(py);
        let raw_attr = stage_ref.getattr("raw_dict")?;
        let raw_dict = raw_attr.cast::<PyDict>().ok();

        let mut name = String::new();
        let mut stage_type = String::new();
        let mut is_parallel_shorthand = false;
        let mut namespace = None;

        if let Some(rd) = raw_dict {
            name = rd
                .get_item("name")?
                .and_then(|v| v.extract::<String>().ok())
                .unwrap_or_default();
            stage_type = rd
                .get_item("type")?
                .and_then(|v| v.extract::<String>().ok())
                .unwrap_or_default();
            is_parallel_shorthand = rd.contains("parallel")?;
            namespace = match rd.get_item("namespace")? {
                Some(v) => {
                    let s: String = v.extract::<String>().map_err(|_| {
                        pyo3::exceptions::PyValueError::new_err("namespace value must be a string")
                    })?;
                    Some(s)
                }
                None => None,
            };
        }

        let mut body = Vec::new();
        if let Ok(body_attr) = stage_ref.getattr("body") {
            if let Ok(body_list) = body_attr.cast::<PyList>() {
                let mut body_stages: Vec<Py<PyAny>> = Vec::new();
                for item in body_list.iter() {
                    body_stages.push(item.extract::<Py<PyAny>>()?);
                }
                body = py_stages_to_namespace_nodes(py, &body_stages)?;
            }
        }

        nodes.push(core_loader::NamespaceNode {
            name,
            stage_type,
            is_parallel_shorthand,
            namespace,
            namespace_path: String::new(),
            body,
        });
    }
    Ok(nodes)
}

fn apply_namespace_paths(
    py: Python<'_>,
    stages: &[Py<PyAny>],
    nodes: &[core_loader::NamespaceNode],
) -> PyResult<()> {
    for (stage, node) in stages.iter().zip(nodes.iter()) {
        let stage_ref = stage.bind(py);
        stage_ref.setattr("namespace_path", &node.namespace_path)?;
        if !node.body.is_empty() {
            if let Ok(body_attr) = stage_ref.getattr("body") {
                if let Ok(body_list) = body_attr.cast::<PyList>() {
                    let mut body_stages: Vec<Py<PyAny>> = Vec::new();
                    for item in body_list.iter() {
                        body_stages.push(item.extract::<Py<PyAny>>()?);
                    }
                    apply_namespace_paths(py, &body_stages, &node.body)?;
                }
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

    // --- namespace helpers ---

    fn make_mock_stage(
        py: Python<'_>,
        raw_dict: &Bound<'_, PyDict>,
        body: Vec<Py<PyAny>>,
    ) -> Py<PyAny> {
        let globals = PyDict::new(py);
        let code = std::ffi::CString::new(
            "class _MockStage:\n    def __init__(self, raw_dict, body):\n        self.raw_dict = raw_dict\n        self.body = body\n        self.namespace_path = \"\"\n",
        )
        .unwrap();
        py.run(&code, Some(&globals), None).unwrap();
        let cls = globals.get_item("_MockStage").unwrap().unwrap();
        let body_list = PyList::new(py, body).unwrap();
        cls.call1((raw_dict, body_list)).unwrap().extract().unwrap()
    }

    fn ns_dict<'py>(py: Python<'py>, entries: &[(&str, &str)]) -> Bound<'py, PyDict> {
        let d = PyDict::new(py);
        for (k, v) in entries {
            d.set_item(*k, *v).unwrap();
        }
        d
    }

    // --- assign_namespace_paths tests ---

    #[test]
    fn test_namespace_shorthand_parallel() {
        Python::attach(|py| {
            let raw = ns_dict(
                py,
                &[("parallel", ""), ("namespace", "my-ns"), ("name", "mypar")],
            );
            let stage = make_mock_stage(py, &raw, vec![]);
            let stages = vec![stage];
            assign_namespace_paths(py, &stages).unwrap();
            let stage_ref = stages[0].bind(py);
            let path: String = stage_ref
                .getattr("namespace_path")
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(path, "my-ns-1");
        });
    }

    #[test]
    fn test_namespace_explicit_parallel() {
        Python::attach(|py| {
            let raw = ns_dict(
                py,
                &[
                    ("type", "parallel"),
                    ("namespace", "my-ns"),
                    ("name", "mypar"),
                ],
            );
            let stage = make_mock_stage(py, &raw, vec![]);
            let stages = vec![stage];
            assign_namespace_paths(py, &stages).unwrap();
            let stage_ref = stages[0].bind(py);
            let path: String = stage_ref
                .getattr("namespace_path")
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(path, "my-ns-1");
        });
    }

    #[test]
    fn test_namespace_loop() {
        Python::attach(|py| {
            let raw = ns_dict(
                py,
                &[("type", "loop"), ("namespace", "my-ns"), ("name", "myloop")],
            );
            let stage = make_mock_stage(py, &raw, vec![]);
            let stages = vec![stage];
            assign_namespace_paths(py, &stages).unwrap();
            let stage_ref = stages[0].bind(py);
            let path: String = stage_ref
                .getattr("namespace_path")
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(path, "my-ns-1");
        });
    }

    #[test]
    fn test_namespace_sequence() {
        Python::attach(|py| {
            let raw = ns_dict(
                py,
                &[
                    ("type", "sequence"),
                    ("namespace", "my-ns"),
                    ("name", "myseq"),
                ],
            );
            let stage = make_mock_stage(py, &raw, vec![]);
            let stages = vec![stage];
            assign_namespace_paths(py, &stages).unwrap();
            let stage_ref = stages[0].bind(py);
            let path: String = stage_ref
                .getattr("namespace_path")
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(path, "my-ns-1");
        });
    }

    #[test]
    fn test_namespace_rejected_on_agent() {
        Python::attach(|py| {
            let raw = ns_dict(
                py,
                &[("type", "agent"), ("namespace", "bad"), ("name", "myagent")],
            );
            let stage = make_mock_stage(py, &raw, vec![]);
            let stages = vec![stage];
            let err = assign_namespace_paths(py, &stages).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("namespace is only valid on composite stages"),
                "expected composite-only error, got: {msg}"
            );
            assert!(
                msg.contains("myagent"),
                "expected stage name in error, got: {msg}"
            );
        });
    }

    #[test]
    fn test_namespace_rejected_on_exec() {
        Python::attach(|py| {
            let raw = ns_dict(
                py,
                &[("type", "exec"), ("namespace", "bad"), ("name", "myexec")],
            );
            let stage = make_mock_stage(py, &raw, vec![]);
            let stages = vec![stage];
            let err = assign_namespace_paths(py, &stages).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("namespace is only valid on composite stages"),
                "expected composite-only error, got: {msg}"
            );
        });
    }

    #[test]
    fn test_namespace_nested_stack() {
        Python::attach(|py| {
            // Outer loop with namespace "outer", body has a sequence with namespace "inner"
            let inner_raw = ns_dict(
                py,
                &[
                    ("type", "sequence"),
                    ("namespace", "inner"),
                    ("name", "inner-seq"),
                ],
            );
            let inner_stage = make_mock_stage(py, &inner_raw, vec![]);
            let outer_raw = ns_dict(
                py,
                &[
                    ("type", "loop"),
                    ("namespace", "outer"),
                    ("name", "outer-loop"),
                ],
            );
            let outer_stage = make_mock_stage(py, &outer_raw, vec![inner_stage]);
            let stages = vec![outer_stage];
            assign_namespace_paths(py, &stages).unwrap();

            // Outer stage should have "outer-1"
            let outer_ref = stages[0].bind(py);
            let outer_path: String = outer_ref
                .getattr("namespace_path")
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(outer_path, "outer-1");

            // Inner stage should have "outer-1/inner-2"
            let body = outer_ref.getattr("body").unwrap();
            let body_list = body.cast::<PyList>().unwrap();
            let inner_ref = body_list.get_item(0).unwrap();
            let inner_path: String = inner_ref
                .getattr("namespace_path")
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(inner_path, "outer-1/inner-2");
        });
    }

    #[test]
    fn test_namespace_no_namespace_empty_path() {
        Python::attach(|py| {
            let raw = ns_dict(py, &[("type", "agent"), ("name", "plain-agent")]);
            let stage = make_mock_stage(py, &raw, vec![]);
            let stages = vec![stage];
            assign_namespace_paths(py, &stages).unwrap();
            let stage_ref = stages[0].bind(py);
            let path: String = stage_ref
                .getattr("namespace_path")
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(path, "");
        });
    }

    #[test]
    fn test_namespace_sibling_counter_increments() {
        Python::attach(|py| {
            let raw1 = ns_dict(
                py,
                &[("type", "loop"), ("namespace", "first"), ("name", "loop1")],
            );
            let raw2 = ns_dict(
                py,
                &[("type", "loop"), ("namespace", "second"), ("name", "loop2")],
            );
            let stage1 = make_mock_stage(py, &raw1, vec![]);
            let stage2 = make_mock_stage(py, &raw2, vec![]);
            let stages = vec![stage1, stage2];
            assign_namespace_paths(py, &stages).unwrap();

            let s1_ref = stages[0].bind(py);
            let p1: String = s1_ref.getattr("namespace_path").unwrap().extract().unwrap();
            assert_eq!(p1, "first-1");

            let s2_ref = stages[1].bind(py);
            let p2: String = s2_ref.getattr("namespace_path").unwrap().extract().unwrap();
            assert_eq!(p2, "second-2");
        });
    }

    #[test]
    fn test_namespace_stack_pops_correctly() {
        Python::attach(|py| {
            // composite with namespace, then a sibling without namespace:
            // the sibling should get an empty path (stack popped after composite)
            let comp_raw = ns_dict(
                py,
                &[("type", "loop"), ("namespace", "only"), ("name", "comp")],
            );
            let agent_raw = ns_dict(py, &[("type", "agent"), ("name", "plain")]);
            let comp_stage = make_mock_stage(py, &comp_raw, vec![]);
            let agent_stage = make_mock_stage(py, &agent_raw, vec![]);
            let stages = vec![comp_stage, agent_stage];
            assign_namespace_paths(py, &stages).unwrap();

            let s0_ref = stages[0].bind(py);
            let p0: String = s0_ref.getattr("namespace_path").unwrap().extract().unwrap();
            assert_eq!(p0, "only-1");

            let s1_ref = stages[1].bind(py);
            let p1: String = s1_ref.getattr("namespace_path").unwrap().extract().unwrap();
            assert_eq!(p1, "");
        });
    }
}
