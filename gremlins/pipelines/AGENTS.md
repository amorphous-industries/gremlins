# gremlins pipeline configuration

This project uses [gremlins](https://github.com/xbrianh/gremlins) for AI-driven background pipelines. Configuration lives in `.gremlins/`.

## `.gremlins/pipelines/*.yaml`

Each YAML file defines a pipeline. The pipeline's name is the filename stem (e.g. `my-pipeline.yaml` → `my-pipeline`). Key fields:

```yaml
default_client: openai:gpt-4o    # provider:model — one of: openai, xai, openrouter, cmd

prompt_dir: ../prompts            # directory bare-name `prompt:` paths resolve against (relative to this YAML; default = YAML dir)

stages:
  - name: <stage-name>
    type: <stage-type>          # agent | exec | plan-gh | implement | verify | review-code | …
    client: <spec>              # optional; overrides default_client for this stage
    prompt: [gremlins:foo.md, foo.md]   # `gremlins:NAME` -> bundled package prompts; bare NAME -> prompt_dir
    options:                    # stage-specific knobs
      check_cmd: "make check"   # verify: command run as lint/type-check gate
      test_cmd:  "make test"    # verify: command run as test gate
```

Stages run in order. A stage can be wrapped in a `parallel:` group to run concurrently.

The `default_client` field is required and sets the AI provider for all stages. Individual stages can override it with their own `client:` field. Supported providers: `openai`, `xai`, `openrouter`, `cmd`.

## `.gremlins/prompts/*.md`

Markdown prompt templates injected into the model's system prompt for the stage that references them. Edit in place — no re-scaffolding needed. Templates may use subdirectories (e.g. `review/detail.md`).
Bundled defaults for these files live under `gremlins/prompts/` in the package.