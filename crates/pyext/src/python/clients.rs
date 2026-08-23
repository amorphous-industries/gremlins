use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use gremlins::clients::backend::{Backend, ClientError, RunParams};
use gremlins::clients::cmd_backend::CmdBackend;
use gremlins::clients::openai_backend::{OpenAiBackend, OpenAiProvider};
use gremlins::clients::protocol::CompletedRun;
use rig_core::providers::openai;

/// Python-exposed RustClient.
#[pyclass]
pub struct RustClient {
    inner: Arc<dyn Backend>,
}

fn map_error(e: ClientError) -> PyErr {
    match e {
        ClientError::Timeout { message } => pyo3::exceptions::PyTimeoutError::new_err(message),
        ClientError::ApiServerError { message } => {
            pyo3::exceptions::PyRuntimeError::new_err(message)
        }
        ClientError::Runtime { message } => pyo3::exceptions::PyRuntimeError::new_err(message),
    }
}

#[pymethods]
impl RustClient {
    #[new]
    #[pyo3(signature = (provider, model, native_block, instructions=None))]
    fn new(
        provider: String,
        model: String,
        native_block: HashMap<String, Vec<String>>,
        instructions: Option<String>,
    ) -> PyResult<Self> {
        let kind = match provider.as_str() {
            "cmd" => {
                let cmd =
                    CmdBackend::new(&model).map_err(pyo3::exceptions::PyValueError::new_err)?;
                return Ok(RustClient {
                    inner: Arc::new(cmd),
                });
            }
            "openai" => OpenAiProvider::OpenAi,
            "xai" => OpenAiProvider::Xai,
            "openrouter" => OpenAiProvider::OpenRouter,
            _ => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unknown provider '{provider}'"
                )));
            }
        };
        let api_key = std::env::var(kind.api_key_env()).map_err(|_| {
            pyo3::exceptions::PyRuntimeError::new_err(format!(
                "{} environment variable is not set",
                kind.api_key_env()
            ))
        })?;
        let client = openai::Client::builder()
            .api_key(rig_core::client::BearerAuth::from(api_key))
            .base_url(kind.base_url())
            .build()
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?
            .completions_api();
        let tool_filter = native_block.get("allowed_tools").cloned();
        Ok(RustClient {
            inner: Arc::new(OpenAiBackend::new(
                kind,
                client,
                model,
                instructions.unwrap_or_default(),
                tool_filter,
            )),
        })
    }

    #[staticmethod]
    fn cmd(command: String) -> PyResult<Self> {
        let backend = CmdBackend::new(&command).map_err(pyo3::exceptions::PyValueError::new_err)?;
        Ok(RustClient {
            inner: Arc::new(backend),
        })
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (prompt, label, model=None, raw_path=None, capture_events=false, on_timeout_prompt=None, max_retries=0, cwd=None, idle_timeout=None, extra_env=None))]
    fn run<'py>(
        &self,
        py: Python<'py>,
        prompt: String,
        label: String,
        model: Option<String>,
        raw_path: Option<PathBuf>,
        capture_events: bool,
        on_timeout_prompt: Option<String>,
        max_retries: usize,
        cwd: Option<PathBuf>,
        idle_timeout: Option<f64>,
        extra_env: Option<HashMap<String, String>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let backend = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let params = RunParams {
                prompt,
                label,
                model,
                raw_path,
                capture_events,
                on_timeout_prompt,
                max_retries,
                cwd,
                idle_timeout,
                extra_env,
            };
            let result = backend.run(params).await.map_err(map_error)?;
            Python::attach(|py| completed_run_to_py(py, &result))
        })
    }

    fn resume<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let backend = self.inner.clone();
        pyo3_async_runtimes::tokio::future_into_py(py, async move {
            let result = backend.resume().await.map_err(map_error)?;
            Python::attach(|py| completed_run_to_py(py, &result))
        })
    }

    fn reap_all(&self) {
        self.inner.reap_all();
    }

    #[getter]
    fn total_cost_usd(&self) -> Option<f64> {
        self.inner.total_cost_usd()
    }
}

fn completed_run_to_py(py: Python<'_>, r: &CompletedRun) -> PyResult<Py<PyAny>> {
    let protocol = py.import("gremlins.clients.protocol")?;
    let cls = protocol.getattr("CompletedRun")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("exit_code", r.exit_code)?;
    if let Some(ref text) = r.text_result {
        kwargs.set_item("text_result", text)?;
    }
    if let Some(ref events) = r.events {
        let py_events = PyList::empty(py);
        let json_mod = py.import("json")?;
        for evt in events {
            let json_str = serde_json::to_string(evt).map_err(|e| {
                pyo3::exceptions::PyValueError::new_err(format!("JSON serialization error: {e}"))
            })?;
            let py_obj = json_mod.call_method1("loads", (json_str,))?;
            py_events.append(py_obj)?;
        }
        kwargs.set_item("events", py_events)?;
    }
    if let Some(cost) = r.cost_usd {
        kwargs.set_item("cost_usd", cost)?;
    }
    Ok(cls.call((), Some(&kwargs))?.into_any().unbind())
}
