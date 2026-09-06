use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use gremlins::core::discovery::DiscoveryError;

pub fn discovery_error_to_pyerr(e: DiscoveryError) -> pyo3::PyErr {
    pyo3::exceptions::PyFileNotFoundError::new_err(e.to_string())
}

pub fn pyval_to_serde(obj: &Bound<'_, PyAny>) -> PyResult<serde_yaml::Value> {
    if obj.is_none() {
        Ok(serde_yaml::Value::Null)
    } else if let Ok(s) = obj.extract::<String>() {
        Ok(serde_yaml::Value::String(s))
    } else if let Ok(b) = obj.extract::<bool>() {
        Ok(serde_yaml::Value::Bool(b))
    } else if let Ok(i) = obj.extract::<i64>() {
        Ok(serde_yaml::Value::Number(serde_yaml::Number::from(i)))
    } else if let Ok(d) = obj.cast::<PyDict>() {
        let mut mapping = serde_yaml::Mapping::new();
        for (k, v) in d.iter() {
            let k_str: String = k.extract()?;
            mapping.insert(serde_yaml::Value::String(k_str), pyval_to_serde(&v)?);
        }
        Ok(serde_yaml::Value::Mapping(mapping))
    } else if let Ok(l) = obj.cast::<PyList>() {
        let mut seq = Vec::new();
        for item in l.iter() {
            seq.push(pyval_to_serde(&item)?);
        }
        Ok(serde_yaml::Value::Sequence(seq))
    } else {
        Ok(serde_yaml::Value::String(obj.to_string()))
    }
}
