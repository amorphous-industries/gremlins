<!-- placeholders: verify_output, diff_text -->
The verify step failed. Read the captured output below, then fix the code so
the commands exit 0.

**Steps:**
1. Read the failure output and diff, and fix the actual violations. Fix only the
   specific errors reported in the output below — do not investigate git history,
   diff against past commits, or chase unrelated tests. The failing lines name the
   exact files and locations to fix. Do not skip
   or disable linting rules, formatter directives, or type-check annotations to
   make checks pass. Do not weaken, delete, or change the intent of test
   assertions or fixtures — fix the implementation code instead. Mechanical
   cleanups in test files (import sorting, unused imports, formatting, type
   annotations) are allowed.
2. Self-verify by running:
   ```bash
   source .venv/bin/activate && make check && make test
   ```
3. If the check passes, stage the changed files by name and create a single git
   commit titled 'Fix failing checks'. Do not push.
4. If the check fails, return to step 1 and iterate.

---

**Output:**

```
{verify_output}
```

---

**Current diff (uncommitted changes):**

```
{diff_text}
```