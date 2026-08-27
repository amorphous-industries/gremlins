You are composing a GitHub pull request for changes on a detached HEAD.

`{out_files}` is a JSON mapping from logical name to the filename you must write. Write each output file using the mapped filename.

| Logical name     | Content |
|------------------|---------|
| `pr-branch.txt` | One line — the branch name to push. If `{plan_source_issue_number}` is non-empty, use `issue-{plan_source_issue_number}-<short-slug>`; otherwise use a short descriptive slug based on the changes. |
| `pr-title.txt`  | One line — the PR title. |
| `pr-body.md`    | The PR body in markdown. If `{plan_source_issue_number}` is non-empty, include `Closes #{plan_source_issue_number}` on its own line. If `{plan_source_issue_number}` is empty, do NOT include any 'Closes' or 'Fixes' line. |

The PR will target `{base_ref_to_open_pr}`. Do NOT push or call `gh pr create` — another stage handles that.
