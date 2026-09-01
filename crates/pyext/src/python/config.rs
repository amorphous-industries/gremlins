use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::PyDict;

use gremlins::config::{self, Config};

/// Python-exposed Config wrapper.
#[pyclass(name = "PyConfig")]
pub struct PyConfig {
    inner: Arc<Config>,
}

#[pymethods]
impl PyConfig {
    #[new]
    fn new() -> Self {
        PyConfig {
            inner: Arc::new(Config::default()),
        }
    }

    fn load(&mut self) -> PyResult<()> {
        self.inner = Arc::new(Config::load());
        Ok(())
    }

    #[getter]
    fn default_client(&self) -> Option<&str> {
        self.inner.default_client()
    }

    fn default_client_by_stage(&self) -> (HashMap<String, String>, HashMap<String, String>) {
        let (exact, prefix) = self.inner.default_client_by_stage();
        (exact.clone(), prefix.clone())
    }

    #[getter]
    fn raw(&self, py: Python<'_>) -> PyResult<PyObject> {
        let raw = self.inner.raw();
        let dict = PyDict::new(py);
        for (k, v) in raw {
            let py_val = serde_json_to_py(py, v)?;
            dict.set_item(k, py_val)?;
        }
        Ok(dict.into())
    }
}

fn serde_json_to_py(py: Python<'_>, value: &serde_json::Value) -> PyResult<PyObject> {
    match value {
        serde_json::Value::Null => Ok(py.None()),
        serde_json::Value::Bool(b) => Ok(b.to_object(py)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.to_object(py))
            } else if let Some(f) = n.as_f64() {
                Ok(f.to_object(py))
            } else {
                Ok(py.None())
            }
        }
        serde_json::Value::String(s) => Ok(s.to_object(py)),
        serde_json::Value::Array(arr) => {
            let list: Vec<PyObject> = arr
                .iter()
                .map(|v| serde_json_to_py(py, v))
                .collect::<PyResult<_>>()?;
            Ok(list.to_object(py))
        }
        serde_json::Value::Object(obj) => {
            let dict = PyDict::new(py);
            for (k, v) in obj {
                dict.set_item(k, serde_json_to_py(py, v)?)?;
            }
            Ok(dict.into())
        }
    }
}

// ---------------------------------------------------------------------------
// Module-level functions — thin delegation to Rust global
// ---------------------------------------------------------------------------

#[pyfunction]
fn init() -> PyResult<()> {
    config::init_global();
    Ok(())
}

#[pyfunction]
fn get_config() -> PyResult<PyConfig> {
    Ok(PyConfig {
        inner: config::global_config(),
    })
}

#[pyfunction]
fn clear() -> PyResult<()> {
    config::clear_global();
    Ok(())
}

// ---------------------------------------------------------------------------
// Path functions
// ---------------------------------------------------------------------------

#[pyfunction]
#[pyo3(signature = (config=None))]
fn state_root(config: Option<&PyConfig>) -> PyResult<String> {
    if let Some(cfg) = config {
        let overrides = cfg.inner.path_overrides();
        Ok(gremlins::config::resolve_state_root(Some(overrides))
            .to_string_lossy()
            .to_string())
    } else {
        Ok(config::state_root().to_string_lossy().to_string())
    }
}

#[pyfunction]
#[pyo3(signature = (config=None))]
fn work_root(config: Option<&PyConfig>) -> PyResult<String> {
    if let Some(cfg) = config {
        let overrides = cfg.inner.path_overrides();
        Ok(gremlins::config::resolve_work_root(Some(overrides))
            .to_string_lossy()
            .to_string())
    } else {
        Ok(config::work_root().to_string_lossy().to_string())
    }
}

#[pyfunction]
#[pyo3(signature = (config=None))]
fn user_config_root(config: Option<&PyConfig>) -> PyResult<String> {
    if let Some(cfg) = config {
        let overrides = cfg.inner.path_overrides();
        Ok(gremlins::config::user_config_root_inner(Some(overrides))
            .to_string_lossy()
            .to_string())
    } else {
        Ok(config::user_config_root().to_string_lossy().to_string())
    }
}

#[pyfunction]
#[pyo3(signature = (config=None))]
fn project_root(config: Option<&PyConfig>) -> PyResult<String> {
    if let Some(cfg) = config {
        let overrides = cfg.inner.path_overrides();
        Ok(gremlins::config::resolve_project_root(Some(overrides))
            .to_string_lossy()
            .to_string())
    } else {
        Ok(config::project_root().to_string_lossy().to_string())
    }
}

#[pyfunction]
#[pyo3(signature = (project_root, config=None))]
fn project_overlay_dir(project_root: &str, config: Option<&PyConfig>) -> PyResult<String> {
    let pr = PathBuf::from(project_root);
    if let Some(cfg) = config {
        let overrides = cfg.inner.path_overrides();
        Ok(
            gremlins::config::resolve_project_overlay_dir(Some(overrides), &pr)
                .to_string_lossy()
                .to_string(),
        )
    } else {
        Ok(config::project_overlay_dir(&pr)
            .to_string_lossy()
            .to_string())
    }
}

#[pyfunction]
#[pyo3(signature = (gremlin_id=None, config=None))]
fn scratch_root(gremlin_id: Option<&str>, config: Option<&PyConfig>) -> PyResult<String> {
    if let Some(cfg) = config {
        let overrides = cfg.inner.path_overrides();
        Ok(
            gremlins::config::resolve_scratch_root(Some(overrides), gremlin_id)
                .to_string_lossy()
                .to_string(),
        )
    } else {
        Ok(config::scratch_root(gremlin_id)
            .to_string_lossy()
            .to_string())
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn register_config_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyConfig>()?;
    m.add_function(wrap_pyfunction!(init, m)?)?;
    m.add_function(wrap_pyfunction!(get_config, m)?)?;
    m.add_function(wrap_pyfunction!(clear, m)?)?;
    m.add_function(wrap_pyfunction!(state_root, m)?)?;
    m.add_function(wrap_pyfunction!(work_root, m)?)?;
    m.add_function(wrap_pyfunction!(user_config_root, m)?)?;
    m.add_function(wrap_pyfunction!(project_root, m)?)?;
    m.add_function(wrap_pyfunction!(project_overlay_dir, m)?)?;
    m.add_function(wrap_pyfunction!(scratch_root, m)?)?;
    Ok(())
}