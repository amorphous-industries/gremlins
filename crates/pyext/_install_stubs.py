"""Install .py source stubs alongside the .so so pyright finds module sources."""

import pathlib
import shutil

import _gremlins_core

pkg_dir = pathlib.Path(__file__).resolve().parent / "_gremlins_core"
site_dir = pathlib.Path(_gremlins_core.__file__).parent

for src in sorted(pkg_dir.rglob("*.py")):
    rel = src.relative_to(pkg_dir)
    dst = site_dir / rel
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dst)
    print(f"  installed {rel}")
for src in sorted(pkg_dir.rglob("*.pyi")):
    rel = src.relative_to(pkg_dir)
    dst = site_dir / rel
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dst)
    print(f"  installed {rel}")
