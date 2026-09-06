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

/// Compile-time map: prompt path → raw markdown/text content.
pub static PROMPTS: Map<&'static str, &'static str> = phf_map! {
    "address.md"                                => include_str!("../../../../gremlins/prompts/address.md"),
    "analyze.md"                                => include_str!("../../../../gremlins/prompts/analyze.md"),
    "assistant/setup.md"                        => include_str!("../../../../gremlins/prompts/assistant/setup.md"),
    "bail_section.md"                           => include_str!("../../../../gremlins/prompts/bail_section.md"),
    "bail_section_fix.md"                       => include_str!("../../../../gremlins/prompts/bail_section_fix.md"),
    "ci_fix.md"                                 => include_str!("../../../../gremlins/prompts/ci_fix.md"),
    "code_style.md"                             => include_str!("../../../../gremlins/prompts/code_style.md"),
    "github_address_pull_request_reviews.md"    => include_str!("../../../../gremlins/prompts/github_address_pull_request_reviews.md"),
    "github_open_pull_request.md"               => include_str!("../../../../gremlins/prompts/github_open_pull_request.md"),
    "github_review_pull_request.md"             => include_str!("../../../../gremlins/prompts/github_review_pull_request.md"),
    "handoff.md"                                => include_str!("../../../../gremlins/prompts/handoff.md"),
    "handoff_sanitize.md"                       => include_str!("../../../../gremlins/prompts/handoff_sanitize.md"),
    "handoff_spec_section.md"                   => include_str!("../../../../gremlins/prompts/handoff_spec_section.md"),
    "handoff_style_section.md"                  => include_str!("../../../../gremlins/prompts/handoff_style_section.md"),
    "implement_gh.md"                           => include_str!("../../../../gremlins/prompts/implement_gh.md"),
    "implement_local.md"                        => include_str!("../../../../gremlins/prompts/implement_local.md"),
    "plan.md"                                   => include_str!("../../../../gremlins/prompts/plan.md"),
    "plan_gh.md"                                => include_str!("../../../../gremlins/prompts/plan_gh.md"),
    "review/chain.md"                           => include_str!("../../../../gremlins/prompts/review/chain.md"),
    "review/detail.md"                          => include_str!("../../../../gremlins/prompts/review/detail.md"),
    "review/recipe.md"                          => include_str!("../../../../gremlins/prompts/review/recipe.md"),
    "verify_fix.md"                             => include_str!("../../../../gremlins/prompts/verify_fix.md"),
};

/// Compile-time map: recipe name → raw YAML content.
pub static RECIPES: Map<&'static str, &'static str> = phf_map! {
    "github_discover_repo"          => include_str!("../../../../gremlins/recipes/stages/github_discover_repo.yaml"),
    "github_open_pr"                => include_str!("../../../../gremlins/recipes/stages/github_open_pr.yaml"),
    "github_push_to_pr_branch"      => include_str!("../../../../gremlins/recipes/stages/github_push_to_pr_branch.yaml"),
    "github_request_copilot_review" => include_str!("../../../../gremlins/recipes/stages/github_request_copilot_review.yaml"),
    "github_wait_ci"                => include_str!("../../../../gremlins/recipes/stages/github_wait_ci.yaml"),
    "github_wait_copilot"           => include_str!("../../../../gremlins/recipes/stages/github_wait_copilot.yaml"),
    "handoff"                       => include_str!("../../../../gremlins/recipes/stages/handoff.yaml"),
    "implement"                     => include_str!("../../../../gremlins/recipes/stages/implement.yaml"),
    "plan_gh"                       => include_str!("../../../../gremlins/recipes/stages/plan_gh.yaml"),
    "plan"                          => include_str!("../../../../gremlins/recipes/stages/plan.yaml"),
    "review_code"                   => include_str!("../../../../gremlins/recipes/stages/review_code.yaml"),
    "verify"                        => include_str!("../../../../gremlins/recipes/stages/verify.yaml"),
};
