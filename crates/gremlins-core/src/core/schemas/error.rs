use thiserror::Error;

#[derive(Error, Debug)]
pub enum SchemaError {
    #[error("{0}")]
    Generic(String),

    #[error("include cycle detected: {0}")]
    IncludeCycle(String),

    #[error("bundled recipe not found: {name}; available: {available}")]
    BundledRecipeNotFound { name: String, available: String },

    #[error("prompt file not found: {path}")]
    PromptFileNotFound { path: String },

    #[error("prompt file is empty: {path}")]
    PromptFileEmpty { path: String },

    #[error("stage-definition {name}: {msg}")]
    StageDef { name: String, msg: String },

    #[error("stage {name}: {msg}")]
    Stage { name: String, msg: String },

    #[error("input source {name}: {msg}")]
    InputSource { name: String, msg: String },

    #[error("pipeline is missing 'default_client' — every pipeline must declare one")]
    MissingDefaultClient,

    #[error("pipeline file not found: {path}")]
    PipelineFileNotFound { path: String },

    #[error("YAML parse error in {label}: {msg}")]
    YamlParse { label: String, msg: String },

    #[error("expected a YAML mapping in {label}, got {got}")]
    YamlNotMapping { label: String, got: String },
}

impl From<SchemaError> for pyo3::PyErr {
    fn from(e: SchemaError) -> Self {
        use pyo3::exceptions::{PyFileNotFoundError, PyRuntimeError, PyValueError};
        match &e {
            SchemaError::PipelineFileNotFound { .. } | SchemaError::PromptFileNotFound { .. } => {
                PyFileNotFoundError::new_err(e.to_string())
            }
            SchemaError::IncludeCycle(_)
            | SchemaError::BundledRecipeNotFound { .. }
            | SchemaError::PromptFileEmpty { .. }
            | SchemaError::StageDef { .. }
            | SchemaError::Stage { .. }
            | SchemaError::InputSource { .. }
            | SchemaError::MissingDefaultClient
            | SchemaError::YamlParse { .. }
            | SchemaError::YamlNotMapping { .. } => PyValueError::new_err(e.to_string()),
            SchemaError::Generic(_) => PyRuntimeError::new_err(e.to_string()),
        }
    }
}

impl From<std::io::Error> for SchemaError {
    fn from(e: std::io::Error) -> Self {
        SchemaError::Generic(e.to_string())
    }
}

impl From<serde_yaml::Error> for SchemaError {
    fn from(e: serde_yaml::Error) -> Self {
        SchemaError::YamlParse {
            label: "yaml".to_string(),
            msg: e.to_string(),
        }
    }
}
