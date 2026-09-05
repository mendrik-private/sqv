#!/usr/bin/env python3
"""Run the permanent regressions created from the 2026-09-06 audit."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main() -> int:
    repo = Path(__file__).resolve().parents[2]
    environment = os.environ.copy()
    with tempfile.TemporaryDirectory(prefix="sqview-audit-state-") as state:
        environment["XDG_DATA_HOME"] = str(Path(state) / "data")
        environment["XDG_CONFIG_HOME"] = str(Path(state) / "config")
        command = ["cargo", "test", "--locked", "--workspace", "--all-targets"]
        print("Running permanent audit regressions and the full test suite.", flush=True)
        return subprocess.run(command, cwd=repo, env=environment, check=False).returncode


if __name__ == "__main__":
    sys.exit(main())
