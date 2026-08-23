use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyType};

#[pyclass(name = "InputSource", from_py_object)]
#[derive(Clone)]
pub struct InputSource {
    #[pyo3(get, set)]
    pub name: String,
    #[pyo3(get, set)]
    pub types: Vec<String>,
    #[pyo3(get, set)]
    pub optional: bool,
}

#[pymethods]
impl InputSource {
    #[new]
    fn new(name: String, types: Vec<String>, optional: bool) -> PyResult<Self> {
        if types.is_empty() {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "input source {name:?}: types list must not be empty"
            )));
        }
        let valid_types: std::collections::HashSet<&str> =
            ["filepath", "string"].iter().copied().collect();
        for t in &types {
            if !valid_types.contains(t.as_str()) {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "input source {name:?}: unknown type {t:?}. Supported types: filepath, string"
                )));
            }
        }
        Ok(InputSource {
            name,
            types,
            optional,
        })
    }
}

#[pyclass(name = "InputSources", from_py_object)]
#[derive(Clone)]
pub struct InputSources {
    #[pyo3(get)]
    sources: HashMap<String, InputSource>,
}

#[pymethods]
impl InputSources {
    #[new]
    fn new(sources: Option<HashMap<String, InputSource>>) -> Self {
        InputSources {
            sources: sources.unwrap_or_default(),
        }
    }

    #[classmethod]
    fn from_yaml(_cls: &Bound<'_, PyType>, raw: &Bound<'_, PyDict>) -> PyResult<Self> {
        let mut sources: HashMap<String, InputSource> = HashMap::new();
        for (key, entry) in raw.iter() {
            let key: String = key.extract()?;
            let entry_dict: &Bound<'_, PyDict> = entry.cast().map_err(|_| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "input source {key:?}: expected a mapping, got {}",
                    entry
                        .get_type()
                        .name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| "?".to_string())
                ))
            })?;

            let type_raw = entry_dict.get_item("type")?.ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "input source {key:?}: missing required 'type' field"
                ))
            })?;

            let types: Vec<String> = if let Ok(s) = type_raw.extract::<String>() {
                vec![s]
            } else if let Ok(list) = type_raw.cast::<PyList>() {
                if list.is_empty() {
                    return Err(pyo3::exceptions::PyValueError::new_err(format!(
                        "input source {key:?}: type list must not be empty"
                    )));
                }
                let mut result = Vec::new();
                for item in list.iter() {
                    let t: String = item.extract().map_err(|_| {
                        pyo3::exceptions::PyValueError::new_err(format!(
                            "input source {key:?}: all type entries must be strings"
                        ))
                    })?;
                    result.push(t);
                }
                result
            } else {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "input source {key:?}: 'type' must be a string or list of strings, got {}",
                    type_raw
                        .get_type()
                        .name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| "?".to_string())
                )));
            };

            let optional: bool = entry_dict
                .get_item("optional")?
                .and_then(|v| v.extract::<bool>().ok())
                .unwrap_or(false);

            let valid_types: std::collections::HashSet<&str> =
                ["filepath", "string"].iter().copied().collect();
            for t in &types {
                if !valid_types.contains(t.as_str()) {
                    return Err(pyo3::exceptions::PyValueError::new_err(format!(
                        "input source {key:?}: unknown type {t:?}. Supported types: filepath, string"
                    )));
                }
            }

            sources.insert(
                key.clone(),
                InputSource {
                    name: key,
                    types,
                    optional,
                },
            );
        }
        Ok(InputSources { sources })
    }

    fn get(&self, key: &str) -> Option<InputSource> {
        self.sources.get(key).cloned()
    }

    fn all_sources(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.sources.keys().cloned().collect();
        keys.sort();
        keys
    }

    fn required_sources(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .sources
            .iter()
            .filter(|(_, src)| !src.optional)
            .map(|(k, _)| k.clone())
            .collect();
        keys.sort();
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_source_empty_types() {
        let result = InputSource::new("test".to_string(), vec![], false);
        assert!(result.is_err());
    }

    #[test]
    fn test_input_source_unknown_type() {
        let result = InputSource::new("test".to_string(), vec!["bad".to_string()], false);
        assert!(result.is_err());
    }

    #[test]
    fn test_input_source_valid() {
        let result = InputSource::new("test".to_string(), vec!["filepath".to_string()], true);
        assert!(result.is_ok());
        let src = result.unwrap();
        assert_eq!(src.name, "test");
        assert!(src.optional);
    }

    #[test]
    fn test_input_sources_from_yaml() {
        Python::attach(|py| {
            let raw = PyDict::new(py);
            let entry = PyDict::new(py);
            entry.set_item("type", "filepath").unwrap();
            entry.set_item("optional", true).unwrap();
            raw.set_item("my_input", entry).unwrap();

            let result = InputSources::from_yaml(&py.get_type::<InputSources>(), &raw).unwrap();
            assert_eq!(result.all_sources(), vec!["my_input"]);
            assert!(result.required_sources().is_empty());
        });
    }
}
