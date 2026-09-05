# Complexity review

Review this PR with one job: find unnecessary complexity. Ignore style, naming, and bugs unless they reveal a complexity problem — other reviewers cover those.

The plan for this change is:

{plan}

Review against this plan.

## Step 1: Gather PR information

Fetch PR metadata:

```
gh pr view {pr} --json number,title,body,author,baseRefName,headRefName
```

The changed files are listed below. For each file you want to review, fetch the diff with:
`gh pr diff {pr} -- <file>`

{diff}

## Step 2: Review for complexity

Flag, with a concrete suggestion to remove or simplify each:

- **Speculative generality**: abstractions, options, hooks, or extension points with no current caller that needs them. Three similar lines beat a premature abstraction.
- **Backward-compat scaffolding**: aliases, re-exports, deprecation shims, `# removed` markers, dual code paths kept "just in case". This codebase has no external consumers — rename and delete in place.
- **Defensive code at internal boundaries**: try/except that catches what can't happen, validation of values produced by trusted internal code, fallbacks for impossible states.
- **Indirection without payoff**: factories, wrappers, base classes, or helpers that add a layer without removing one. Inheritance where composition would do. Any inheritance hierarchy more than one level deep.
- **Long functions**: if a function doesn't fit on a screen, it's too long. Suggest a split.
- **Configuration knobs nobody asked for**: flags, settings, env vars added "for flexibility" with one caller.
- **Comments that narrate the *what***: if the name already says it, delete the comment. Keep only comments that explain a non-obvious *why*.
- **Dead or unreachable code**: branches that can't fire, parameters never read, returns never used.

## Step 3: Build the review

Construct a JSON body for the GitHub pull request review API. The format is:

```json
{{
  "event": "COMMENT",
  "body": "Overall summary of the complexity review",
  "comments": [
    {{
      "path": "relative/file/path",
      "line": <line_number_in_the_new_file>,
      "side": "RIGHT",
      "body": "Comment text (markdown supported)"
    }}
  ]
}}
```

Rules for the review:
- `event` must be `"COMMENT"` (not APPROVE or REQUEST_CHANGES — leave that decision to a human)
- `line` is the line number in the **new version** of the file (the right side of the diff)
- `side` should always be `"RIGHT"`
- Each comment `body` should include: (1) what's unnecessary, (2) the simpler form, (3) one sentence on why the simpler form is safe here
- The top-level `body` is a concise summary (2-4 sentences) of the overall complexity assessment

If the PR is already tight, write a summary body saying so explicitly and set `comments` to `[]`. Do not invent findings to look thorough.

## Step 4: Post the review

Use `gh api` to submit the review. Get the repo owner/name from the PR metadata or by running `gh repo view --json nameWithOwner -q .nameWithOwner`.

```
gh api repos/{{owner}}/{{repo}}/pulls/{{number}}/reviews --input /dev/stdin <<< "$JSON"
```

Write the JSON to a temp file if it's large, then pass it via `--input`.

After posting, print a link to the PR so the user can see the review.