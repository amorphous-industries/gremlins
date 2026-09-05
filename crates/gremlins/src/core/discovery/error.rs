use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DiscoveryError {
    #[error("pipeline {name:?} not found; available: {available}")]
    Name { name: String, available: String },

    #[error("pipeline file not found: {path}")]
    File { path: PathBuf },

    #[error("pipeline {name:?} not found in {dirs} or bundled pipelines")]
    Path { name: String, dirs: String },
}
