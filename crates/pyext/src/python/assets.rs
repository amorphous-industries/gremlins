use pyo3::prelude::*;

#[pyfunction]
fn load_bundled_prompt(name: &str) -> PyResult<String> {
    gremlins::assets::PROMPTS
        .get(name)
        .map(|s| s.to_string())
        .ok_or_else(|| {
            pyo3::exceptions::PyFileNotFoundError::new_err(format!(
                "bundled prompt not found: {name}"
            ))
        })
}

#[pyfunction]
fn list_bundled_prompts() -> Vec<String> {
    let mut keys: Vec<_> = gremlins::assets::PROMPTS
        .keys()
        .map(|k| k.to_string())
        .collect();
    keys.sort();
    keys
}

pub fn register_assets_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let assets_mod = PyModule::new(m.py(), "assets")?;
    assets_mod.add_function(wrap_pyfunction!(load_bundled_prompt, &assets_mod)?)?;
    assets_mod.add_function(wrap_pyfunction!(list_bundled_prompts, &assets_mod)?)?;
    m.add_submodule(&assets_mod)?;

    let modules = m.py().import("sys")?.getattr("modules")?;
    modules.set_item("_gremlins_core.assets", &assets_mod)?;
    Ok(())
}