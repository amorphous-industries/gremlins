use phf::{phf_map, Map};

/// Compile-time map: prompt path → raw markdown/text content.
pub static PROMPTS: Map<&'static str, &'static str> = phf_map! {
    "address.md"                                => include_str!("data/prompts/address.md"),
    "analyze.md"                                => include_str!("data/prompts/analyze.md"),
    "assistant/setup.md"                        => include_str!("data/prompts/assistant/setup.md"),
    "bail_section.md"                           => include_str!("data/prompts/bail_section.md"),
    "bail_section_fix.md"                       => include_str!("data/prompts/bail_section_fix.md"),
    "ci_fix.md"                                 => include_str!("data/prompts/ci_fix.md"),
    "code_style.md"                             => include_str!("data/prompts/code_style.md"),
    "github_address_pull_request_reviews.md"    => include_str!("data/prompts/github_address_pull_request_reviews.md"),
    "github_open_pull_request.md"               => include_str!("data/prompts/github_open_pull_request.md"),
    "github_review_pull_request.md"             => include_str!("data/prompts/github_review_pull_request.md"),
    "handoff.md"                                => include_str!("data/prompts/handoff.md"),
    "handoff_sanitize.md"                       => include_str!("data/prompts/handoff_sanitize.md"),
    "handoff_spec_section.md"                   => include_str!("data/prompts/handoff_spec_section.md"),
    "handoff_style_section.md"                  => include_str!("data/prompts/handoff_style_section.md"),
    "implement_gh.md"                           => include_str!("data/prompts/implement_gh.md"),
    "implement_local.md"                        => include_str!("data/prompts/implement_local.md"),
    "plan.md"                                   => include_str!("data/prompts/plan.md"),
    "plan_gh.md"                                => include_str!("data/prompts/plan_gh.md"),
    "review/chain.md"                           => include_str!("data/prompts/review/chain.md"),
    "review/detail.md"                          => include_str!("data/prompts/review/detail.md"),
    "review/recipe.md"                          => include_str!("data/prompts/review/recipe.md"),
    "verify_fix.md"                             => include_str!("data/prompts/verify_fix.md"),
};

/// Compile-time map: recipe name → raw YAML content.
pub static RECIPES: Map<&'static str, &'static str> = phf_map! {
    "github_discover_repo"          => include_str!("data/stages/github_discover_repo.yaml"),
    "github_open_pr"                => include_str!("data/stages/github_open_pr.yaml"),
    "github_push_to_pr_branch"      => include_str!("data/stages/github_push_to_pr_branch.yaml"),
    "github_request_copilot_review" => include_str!("data/stages/github_request_copilot_review.yaml"),
    "github_wait_ci"                => include_str!("data/stages/github_wait_ci.yaml"),
    "github_wait_copilot"           => include_str!("data/stages/github_wait_copilot.yaml"),
    "handoff"                       => include_str!("data/stages/handoff.yaml"),
    "implement"                     => include_str!("data/stages/implement.yaml"),
    "plan_gh"                       => include_str!("data/stages/plan_gh.yaml"),
    "plan"                          => include_str!("data/stages/plan.yaml"),
    "review_code"                   => include_str!("data/stages/review_code.yaml"),
    "verify"                        => include_str!("data/stages/verify.yaml"),
};
