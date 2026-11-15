"""PostToolUse hook: format a Rust file right after Claude edits it.

Reads the hook's JSON on stdin; when the edited file is a .rs file, runs rustfmt on that
file alone with the workspace's edition. Never fails the edit: errors are ignored.
"""

import json
import subprocess
import sys


def main() -> int:
    try:
        data = json.load(sys.stdin)
    except ValueError:
        return 0
    path = (data.get("tool_input") or {}).get("file_path") or ""
    if path.endswith(".rs"):
        try:
            subprocess.run(["rustfmt", "--edition", "2021", path],
                           capture_output=True, timeout=30, check=False)
        except (OSError, subprocess.SubprocessError):
            pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
