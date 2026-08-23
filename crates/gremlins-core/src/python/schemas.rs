use std::path::PathBuf;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::core::schemas;

#[pyfunction]
fn parse_stage(d: &Bound<'_, PyDict>, depth: usize) -> PyResult<Py<PyAny>> {
    schemas::loader::parse_stage(d.py(), d, depth)
}

#[pyfunction]
fn parse_stages(raw: &Bound<'_, PyList>, depth: usize) -> PyResult<Vec<Py<PyAny>>> {
    schemas::loader::parse_stages(raw.py(), raw, depth)
}

#[pyfunction]
fn fill_names(raw_stages: &Bound<'_, PyList>) -> PyResult<()> {
    schemas::loader::fill_names(raw_stages)
}

#[pyfunction]
fn check_duplicate_producers(stages: &Bound<'_, PyList>) -> PyResult<()> {
    schemas::loader::check_duplicate_producers(stages)
}

#[pyfunction]
fn expand_pipeline(
    py: Python<'_>,
    yaml_path: PathBuf,
    project_root: Option<PathBuf>,
    bundled_stage_def_dir: PathBuf,
    bundled_prompt_dir: PathBuf,
    bundled_pipeline_dir: PathBuf,
    resolve_pipeline_name_fn: Py<PyAny>,
) -> PyResult<Py<PyAny>> {
    schemas::preprocess::expand_pipeline(
        py,
        yaml_path,
        project_root,
        bundled_stage_def_dir,
        bundled_prompt_dir,
        bundled_pipeline_dir,
        resolve_pipeline_name_fn.bind(py),
    )
}

pub const GREMLINS_PREFIX: &str = schemas::GREMLINS_PREFIX;

pub fn register_schemas_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let schemas_mod = PyModule::new(m.py(), "schemas")?;

    schemas_mod.add_class::<schemas::inputs::InputSource>()?;
    schemas_mod.add_class::<schemas::inputs::InputSources>()?;
    schemas_mod.add_class::<schemas::pipeline::Pipeline>()?;
    schemas_mod.add("GREMLINS_PREFIX", GREMLINS_PREFIX)?;
    schemas_mod.add_function(wrap_pyfunction!(parse_stage, &schemas_mod)?)?;
    schemas_mod.add_function(wrap_pyfunction!(parse_stages, &schemas_mod)?)?;
    schemas_mod.add_function(wrap_pyfunction!(fill_names, &schemas_mod)?)?;
    schemas_mod.add_function(wrap_pyfunction!(check_duplicate_producers, &schemas_mod)?)?;
    schemas_mod.add_function(wrap_pyfunction!(expand_pipeline, &schemas_mod)?)?;

    m.add_submodule(&schemas_mod)?;

    let modules = m.py().import("sys")?.getattr("modules")?;
    modules.set_item("_gremlins_core.schemas", &schemas_mod)?;

    Ok(())
}
