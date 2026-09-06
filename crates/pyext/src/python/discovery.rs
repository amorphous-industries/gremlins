use pyo3::prelude::*;

use crate::convert::discovery_error_to_pyerr;
use gremlins::config;
use gremlins::core::discovery;

#[pyfunction]
fn list_pipelines() -> Vec<(String, std::path::PathBuf)> {
    discovery::list_pipelines(config::project_root())
}

#[pyfunction]
fn resolve_pipeline_name(name: &str) -> PyResult<std::path::PathBuf> {
    discovery::resolve_pipeline_name(name, config::project_root()).map_err(discovery_error_to_pyerr)
}

#[pyfunction]
fn resolve_pipeline_path(name_or_path: &str) -> PyResult<std::path::PathBuf> {
    discovery::resolve_pipeline_path(name_or_path, config::project_root())
        .map_err(discovery_error_to_pyerr)
}

pub fn register_discovery_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let discovery_mod = PyModule::new(m.py(), "discovery")?;

    discovery_mod.add_function(wrap_pyfunction!(list_pipelines, &discovery_mod)?)?;
    discovery_mod.add_function(wrap_pyfunction!(resolve_pipeline_name, &discovery_mod)?)?;
    discovery_mod.add_function(wrap_pyfunction!(resolve_pipeline_path, &discovery_mod)?)?;

    m.add_submodule(&discovery_mod)?;

    let modules = m.py().import("sys")?.getattr("modules")?;
    modules.set_item("_gremlins_core.discovery", &discovery_mod)?;

    Ok(())
}
