use std::path::PathBuf;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyType};

use crate::schemas::loader;

#[pyclass(name = "Pipeline")]
pub struct Pipeline {
    #[pyo3(get, set)]
    pub name: String,
    #[pyo3(get, set)]
    pub path: PathBuf,
    #[pyo3(get, set)]
    pub stages: Vec<Py<PyAny>>,
    #[pyo3(get, set)]
    pub default_client: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    pub base_ref: String,
    #[pyo3(get, set)]
    pub bootstrap: Option<Py<PyAny>>,
    #[pyo3(get, set)]
    pub land: Option<Py<PyAny>>,
}

#[pymethods]
impl Pipeline {
    #[classmethod]
    fn from_yaml(
        _cls: &Bound<'_, PyType>,
        path: PathBuf,
        bundled_stage_def_dir: PathBuf,
        bundled_prompt_dir: PathBuf,
        bundled_pipeline_dir: PathBuf,
        resolve_pipeline_name_fn: Py<PyAny>,
    ) -> PyResult<Self> {
        let py = _cls.py();

        let path = path.canonicalize().unwrap_or(path);
        if !path.exists() {
            return Err(pyo3::exceptions::PyFileNotFoundError::new_err(format!(
                "pipeline file not found: {}",
                path.display()
            )));
        }

        py.import("gremlins.clients")?;

        let raw = crate::schemas::preprocess::expand_pipeline(
            py,
            path.clone(),
            None,
            bundled_stage_def_dir,
            bundled_prompt_dir,
            bundled_pipeline_dir,
            resolve_pipeline_name_fn.bind(py),
        )?;

        let raw_dict: &Bound<'_, PyDict> = raw.bind(py).cast()?;
        let pipeline_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let default_client: Option<Py<PyAny>> = raw_dict
            .get_item("default_client")?
            .map(|v| {
                let s: String = v.extract()?;
                if s.is_empty() {
                    return Err(pyo3::exceptions::PyValueError::new_err(
                        "default_client must be a non-empty string",
                    ));
                }
                let client_cls = py.import("gremlins.clients.client")?.getattr("Client")?;
                let client: Py<PyAny> = client_cls.call_method1("parse", (s,))?.extract()?;
                Ok(client)
            })
            .transpose()?;

        let base_ref: String = raw_dict
            .get_item("base_ref")?
            .map(|v| {
                let s: String = v.extract()?;
                if s.trim().is_empty() {
                    Err(pyo3::exceptions::PyValueError::new_err(
                        "base_ref must be a non-empty string",
                    ))
                } else {
                    Ok(s.trim().to_string())
                }
            })
            .transpose()?
            .unwrap_or_else(|| "current".to_string());

        let raw_stages_val = raw_dict.get_item("stages")?;
        let empty_list = PyList::empty(py);
        let raw_stages: &Bound<'_, PyList> = match raw_stages_val.as_ref() {
            None => &empty_list,
            Some(ob) if ob.is_none() => &empty_list,
            Some(ob) => ob.cast::<PyList>().map_err(|_| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "'stages' must be a list; got {} type",
                    ob.get_type()
                        .name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| "?".to_string())
                ))
            })?,
        };

        let stages = loader::parse_stages(py, raw_stages, 0)?;

        // Reject the old "inputs" key — declare CLI args under bootstrap.source
        if raw_dict.get_item("inputs")?.is_some() {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "'inputs' is not a valid pipeline key; declare CLI arguments under bootstrap.source"
            ));
        }

        let bootstrap: Option<Py<PyAny>> = raw_dict
            .get_item("bootstrap")?
            .map(|v| {
                if v.is_none() {
                    return Ok::<Option<Py<PyAny>>, pyo3::PyErr>(None);
                }
                let bootstrap_dict: &Bound<'_, PyDict> = v.cast().map_err(|_| {
                    pyo3::exceptions::PyValueError::new_err("'bootstrap' must be a mapping")
                })?;
                let bootstrap_cls = py
                    .import("gremlins.pipeline.bootstrap")?
                    .getattr("Bootstrap")?;
                let bs = bootstrap_cls
                    .call_method1("from_yaml", (bootstrap_dict,))?
                    .extract()?;
                Ok::<Option<Py<PyAny>>, pyo3::PyErr>(Some(bs))
            })
            .transpose()?
            .flatten();

        let land_stage: Option<Py<PyAny>> = raw_dict
            .get_item("land")?
            .map(|v| {
                let land_dict: &Bound<'_, PyDict> = v.cast().map_err(|_| {
                    pyo3::exceptions::PyValueError::new_err("'land' must be a mapping")
                })?;
                let exec_cls = py.import("gremlins.stages.exec")?.getattr("Exec")?;
                let land_stage_dict = PyDict::new(py);
                land_stage_dict.set_item("name", "land")?;
                for (k, v) in land_dict.iter() {
                    land_stage_dict.set_item(k, v)?;
                }
                let stage: Py<PyAny> = exec_cls
                    .call_method1("with_dict", (land_stage_dict,))?
                    .extract()?;
                Ok::<Py<PyAny>, pyo3::PyErr>(stage)
            })
            .transpose()?;

        let stages_list = PyList::new(py, stages.iter().map(|s| s.bind(py)))?;
        loader::check_duplicate_producers(&stages_list)?;

        if default_client.is_none() {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "pipeline is missing 'default_client' — every pipeline must declare one",
            ));
        }

        if let Some(ref dc) = default_client {
            fill_stage_clients(py, &stages, dc)?;
        }

        Ok(Pipeline {
            name: pipeline_name,
            path,
            stages,
            default_client,
            base_ref,
            bootstrap,
            land: land_stage,
        })
    }

    fn uses_loop_handoff(&self, py: Python<'_>) -> bool {
        if self.stages.is_empty() {
            return false;
        }
        let first = &self.stages[0];
        let stage_type: Option<String> = first
            .getattr(py, "type")
            .ok()
            .and_then(|v| v.extract(py).ok());
        if stage_type.as_deref() != Some("loop") {
            return false;
        }
        let body: Option<Vec<Py<PyAny>>> = first
            .getattr(py, "body")
            .ok()
            .and_then(|v| v.extract(py).ok());
        match body {
            Some(children) => children.iter().any(|c| {
                c.getattr(py, "name")
                    .ok()
                    .and_then(|n| n.extract::<String>(py).ok())
                    .is_some_and(|n| n == "handoff")
            }),
            None => false,
        }
    }
}

fn fill_stage_clients(py: Python<'_>, stages: &[Py<PyAny>], default: &Py<PyAny>) -> PyResult<()> {
    for stage in stages {
        let stage_type: String = stage.getattr(py, "type")?.extract(py)?;
        if stage_type != "parallel" {
            let client: Option<Py<PyAny>> = stage.getattr(py, "client")?.extract(py)?;
            if client.is_none() {
                stage.setattr(py, "client", default)?;
            }
        }
        let body: Option<Vec<Py<PyAny>>> = stage
            .getattr(py, "body")
            .ok()
            .and_then(|v| v.extract(py).ok());
        if let Some(body) = body {
            fill_stage_clients(py, &body, default)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uses_loop_handoff_empty() {
        let p = Pipeline {
            name: "test".to_string(),
            path: PathBuf::from("."),
            stages: vec![],
            default_client: None,
            base_ref: "current".to_string(),
            bootstrap: None,
            land: None,
        };
        Python::attach(|py| {
            assert!(!p.uses_loop_handoff(py));
        });
    }
}
