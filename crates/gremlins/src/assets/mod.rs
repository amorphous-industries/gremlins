use phf::{phf_map, Map};

/// Compile-time map: pipeline name → YAML content.
pub static PIPELINES: Map<&'static str, &'static str> = phf_map! {
    "boss"      => include_str!("../../../../gremlins/pipelines/boss.yaml"),
    "gh"        => include_str!("../../../../gremlins/pipelines/gh.yaml"),
    "gh-terse"  => include_str!("../../../../gremlins/pipelines/gh-terse.yaml"),
    "local"     => include_str!("../../../../gremlins/pipelines/local.yaml"),
    "pr-extend" => include_str!("../../../../gremlins/pipelines/pr-extend.yaml"),
};

/// Compile-time map: pipeline name → source file path (for `resolve_pipeline_name`).
pub static PIPELINE_PATHS: Map<&'static str, &'static str> = phf_map! {
    "boss"      => "../gremlins/pipelines/boss.yaml",
    "gh"        => "../gremlins/pipelines/gh.yaml",
    "gh-terse"  => "../gremlins/pipelines/gh-terse.yaml",
    "local"     => "../gremlins/pipelines/local.yaml",
    "pr-extend" => "../gremlins/pipelines/pr-extend.yaml",
};
