use std::path::{Path, PathBuf};

use crate::core::discovery;
use crate::schemas::error::SchemaError;
use crate::schemas::expand::PipelineResolver;

/// Resolves pipeline names to file paths by searching project overlay
/// directories. Since bundled pipelines were removed, only project overlays
/// (`.gremlins/`) are searched.
pub struct BuiltinResolver {
    pub project_root: PathBuf,
}

impl PipelineResolver for BuiltinResolver {
    fn resolve(&self, name: &str, _project_root: &Path) -> Result<PathBuf, SchemaError> {
        discovery::resolve_pipeline_name(name, self.project_root.clone()).map_err(|e| {
            SchemaError::PipelineNotFound {
                name: name.to_string(),
                available: e.to_string(),
            }
        })
    }
}