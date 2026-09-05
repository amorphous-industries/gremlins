use std::hash::{DefaultHasher, Hash, Hasher};

use gremlins::artifacts::uri as rust_uri;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

#[pyclass(name = "Uri", module = "_gremlins_core.artifacts")]
struct Uri {
    inner: rust_uri::Uri,
}

#[pymethods]
impl Uri {
    #[new]
    fn new(scheme: String, path: String) -> Self {
        Uri {
            inner: rust_uri::Uri::new(scheme, path),
        }
    }

    #[staticmethod]
    fn parse(s: &str) -> PyResult<Self> {
        rust_uri::Uri::parse(s)
            .map(|u| Uri { inner: u })
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    #[staticmethod]
    fn parse_or_none(s: &str) -> Option<Self> {
        rust_uri::Uri::parse(s).ok().map(|u| Uri { inner: u })
    }

    #[staticmethod]
    fn is_range(value: &str) -> bool {
        rust_uri::Uri::is_range(value)
    }

    #[getter]
    fn scheme(&self) -> &str {
        &self.inner.scheme
    }

    #[getter]
    fn path(&self) -> &str {
        &self.inner.path
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!(
            "Uri(scheme='{}', path='{}')",
            self.inner.scheme, self.inner.path
        )
    }

    fn __eq__(&self, other: &Uri) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut s = DefaultHasher::new();
        self.inner.hash(&mut s);
        s.finish()
    }
}

pub fn register_artifacts_module(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new(parent.py(), "artifacts")?;
    m.add_class::<Uri>()?;
    parent.add_submodule(&m)?;

    let modules = parent.py().import("sys")?.getattr("modules")?;
    modules.set_item("_gremlins_core.artifacts", &m)?;

    Ok(())
}
