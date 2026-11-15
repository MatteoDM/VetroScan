"""PreToolUse hook: always ask Matteo before a command that drives a real scanner.

Reads the hook's JSON on stdin. When a Bash or PowerShell command runs the hp3770 or
epson4490 tool with a subcommand that moves a carriage, lights a lamp or writes to the
scanner, or runs the hardware tests, it answers "ask", so the command waits for a person
even in auto mode or when an allow rule covers it. Anything else: no output, no effect.
"""

import json
import re
import sys

EPSON = {"scan", "film-scan", "replay", "upload", "recover", "drain", "lamp-off", "lid-lamp"}
HP = {"scan", "tma-scan", "tma-preview", "detect", "init", "park", "forward", "lamp"}


def reason(command: str):
    if re.search(r"--test\s+hardware\b", command):
        return "the hardware tests drive the real scanner"
    for tool, moves in (("epson4490", EPSON), ("hp3770", HP)):
        for match in re.finditer(rf"\b{tool}(?:\.exe)?\b", command):
            words = re.findall(r"[A-Za-z][A-Za-z-]*", command[match.end():])
            hit = next((w for w in words if w in moves), None)
            if hit:
                return f"`{tool} {hit}` drives the real scanner"
    return None


def main() -> int:
    try:
        data = json.load(sys.stdin)
    except ValueError:
        return 0
    command = (data.get("tool_input") or {}).get("command") or ""
    why = reason(command)
    if why:
        print(json.dumps({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "ask",
                "permissionDecisionReason": f"Scanner: {why}. Check the scanner is ready and the combination was run before.",
            }
        }))
    return 0


if __name__ == "__main__":
    sys.exit(main())
