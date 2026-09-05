use phf::{phf_map, Map};

/// Compile-time map: pipeline name → YAML content.
pub static PIPELINES: Map<&'static str, &'static str> = phf_map! {
    "boss"      => include_str!("../../../../gremlins/pipelines/boss.yaml"),
    "gh"        => include_str!("../../../../gremlins/pipelines/gh.yaml"),
    "gh-terse"  => include_str!("../../../../gremlins/pipelines/gh-terse.yaml"),
    "local"     => include_str!("../../../../gremlins/pipelines/local.yaml"),
    "pr-extend" => include_str!("../../../../gremlins/pipelines/pr-extend.yaml"),
};

/// Compile-time map: pipeline name → source file name (for `resolve_pipeline_name`).
/// Joined with a runtime-discovered `bundled_pipelines_dir` to produce the full path.
pub static PIPELINE_PATHS: Map<&'static str, &'static str> = phf_map! {
    "boss"      => "boss.yaml",
    "gh"        => "gh.yaml",
    "gh-terse"  => "gh-terse.yaml",
    "local"     => "local.yaml",
    "pr-extend" => "pr-extend.yaml",
};
