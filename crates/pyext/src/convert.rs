use gremlins::core::discovery::DiscoveryError;

pub fn discovery_error_to_pyerr(e: DiscoveryError) -> pyo3::PyErr {
    pyo3::exceptions::PyFileNotFoundError::new_err(e.to_string())
}