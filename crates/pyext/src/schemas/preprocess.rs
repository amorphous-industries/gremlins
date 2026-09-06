use std::path::PathBuf;

use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyList};

use crate::schemas::error::{into_pyerr, SchemaError};
use gremlins::schemas::expand::{self, PipelineResolver};

struct PyResolver<'a> {
    resolve_pipeline_name_fn: &'a Bound<'a, PyAny>,
}

impl PipelineResolver for PyResolver<'_> {
    fn resolve(&self, name: &str, _project_root: &std::path::Path) -> Result<PathBuf, SchemaError> {
        let result = self.resolve_pipeline_name_fn.call1((name,)).map_err(|e| {
            if e.is_instance_of::<pyo3::exceptions::PyFileNotFoundError>(
                self.resolve_pipeline_name_fn.py(),
            ) {
                SchemaError::PipelineNotFound {
                    name: name.to_string(),
                    available: String::new(),
                }
            } else {
                SchemaError::Generic(e.to_string())
            }
        })?;
        if let Ok(s) = result.extract::<String>() {
            Ok(PathBuf::from(s))
        } else if let Ok(p) = result.extract::<PathBuf>() {
            Ok(p)
        } else {
            let type_name = result
                .get_type()
                .name()
                .map_or_else(|_| "<unknown>".to_string(), |n| n.to_string());
            Err(SchemaError::Generic(format!(
                "pipeline resolver returned {type_name} (expected str or Path)",
            )))
        }
    }
}

pub fn expand_pipeline(
    py: Python<'_>,
    yaml_path: PathBuf,
    project_root: Option<PathBuf>,
    resolve_pipeline_name_fn: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let resolver = PyResolver {
        resolve_pipeline_name_fn,
    };
    let result = expand::expand_pipeline(&yaml_path, project_root.as_deref(), &resolver)
        .map_err(into_pyerr)?;
    serde_yaml_to_py(py, &result)
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
