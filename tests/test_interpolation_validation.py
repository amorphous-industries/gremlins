import tempfile
from pathlib import Path

from _gremlins_core.schemas import Pipeline


def _write_pipeline(content: str) -> Path:
    tmp = tempfile.NamedTemporaryFile(
        mode="w", suffix=".yaml", delete=False, prefix="test_pipeline_"
    )
    tmp.write(content)
    tmp.close()
    return Path(tmp.name)


CLIENT = "openai:gpt-4o"


def test_unused_interpolation_key_raises():
    yaml_content = f"""
default_client: {CLIENT}
stages:
  - name: test
    type: agent
    prompt: |
      Hello world
    interpolation:
      plan: artifact.plan
"""
    path = _write_pipeline(yaml_content)
    with pytest.raises(Exception, match="not referenced"):
        Pipeline.from_yaml(path)


def test_valid_pipeline_passes():
    yaml_content = f"""
default_client: {CLIENT}
stages:
  - name: test
    type: agent
    prompt: |
      Hello {{plan}}
    interpolation:
      plan: artifact.plan
"""
    path = _write_pipeline(yaml_content)
    Pipeline.from_yaml(path)


def test_key_in_prompt_text_passes():
    yaml_content = f"""
default_client: {CLIENT}
stages:
  - name: test
    type: agent
    prompt: |
      Use the {{plan}} to do the thing
    interpolation:
      plan: artifact.plan
"""
    path = _write_pipeline(yaml_content)
    Pipeline.from_yaml(path)


def test_key_in_command_string_passes():
    yaml_content = f"""
default_client: {CLIENT}
stages:
  - name: test
    type: exec
    prompt: |
      run the command
    options:
      cmds:
        - "echo {{plan}}"
    interpolation:
      plan: artifact.plan
"""
    path = _write_pipeline(yaml_content)
    Pipeline.from_yaml(path)


def test_key_in_shell_dollar_form_passes():
    yaml_content = f"""
default_client: {CLIENT}
stages:
  - name: test
    type: exec
    prompt: |
      run the command
    options:
      cmds:
        - "echo $plan"
    interpolation:
      plan: artifact.plan
"""
    path = _write_pipeline(yaml_content)
    Pipeline.from_yaml(path)


def test_key_in_shell_dollar_brace_form_passes():
    yaml_content = f"""
default_client: {CLIENT}
stages:
  - name: test
    type: exec
    prompt: |
      run the command
    options:
      cmds:
        - "echo ${{plan}}"
    interpolation:
      plan: artifact.plan
"""
    path = _write_pipeline(yaml_content)
    Pipeline.from_yaml(path)


def test_multiple_keys_one_unused_raises():
    yaml_content = f"""
default_client: {CLIENT}
stages:
  - name: test
    type: agent
    prompt: |
      Hello {{plan}}
    interpolation:
      plan: artifact.plan
      unused_key: artifact.unused
"""
    path = _write_pipeline(yaml_content)
    with pytest.raises(Exception, match="not referenced"):
        Pipeline.from_yaml(path)
