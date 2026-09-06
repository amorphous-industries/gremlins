Let's refactor the artifact registry:
- Instead of names, the keys of the artifact registry will be URIs
- All artifacts will be files in the `artfiact` directory on disk. 
- URIs will use file type as schmea `json://foo/bar`
- The `session` component should be dropped. It has no meaning.
- `bind:` blocks will follow the same format of `{interpolation_key: URI }` as `interpolate:` blocks
- a new DSL command `content` to indicate when to interpolate the content instead of the filepath. For json, this should be supported: `content(URI, path.to.thing)`.
  - otherwise, artifacts are interpolated as there actual path on the filesystem.
- this should remove the need for `{artifact_dir}` or similar magic values to appear in YAML files

```
required-options:
  - cmds
required-prompt: true

stages:
  - name: verify
    type: loop
    stop_when_exists: done
    max-iterations: "{{options.max_iterations | default(3)}}"
    body:
      - name: cmd
        type: exec
        bind:
          verify_log: "file://session/verify_output.txt"
          done?: "file://session/done"
          exit_code?: "file://session/exit_code"
        options:
          timeout: "{{options.timeout | default(900)}}"
          cmds:
            - rm -f -- "{done}"; ({{options.cmds}}) > "{verify_log}" 2>&1; rc=$?; printf '%d' "$rc" > "{exit_code}"; if [ "$rc" -eq 0 ]; then printf 'done' > "{done}"; fi; true
      - name: fix
        type: agent
        skip_if_exists: "file://session/done"
        prompt:
          - "{{prompt}}"
        interpolation:
          verify_output: content("file://session/verify_output.txt")
```
