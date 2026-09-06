<!-- placeholders: verify_output -->
The verify step failed. Read the captured output below, then fix the code so the commands exit 0.

**First: rule out a stale native extension.**

If the error involves an ImportError from `_gremlins_core` or any of its submodules
(e.g. `cannot import name 'overlay_dirname' from 'config'`), first run
`make install` to rebuild the Rust extension and produce a fresh `.so`.
If that resolves the error, verify again with `make -j8 check && make -j8 test`,
and if that passes — commit the now-passing state and stop. Do not investigate
further; the fix was a stale artifact from a previous Rust change.

**If the error persists after `make install`**, proceed with the following
constraints to fix the code:

- Do not skip or disable linting rules, formatter directives, or type-check annotations to make the check pass — fix the actual violation.
- After fixing, stage the changed files by name and create a git commit titled 'Fix failing checks'. Do not push.

---

**Output:**

```
{verify_output}
```


