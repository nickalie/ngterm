#!/usr/bin/env python3
"""Exercise the installed Codex hook with Git Bash too, without contacting a running ngterm."""

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


def bash_path():
    if os.name == "nt":
        candidate = Path(os.environ.get("ProgramFiles", "C:/Program Files")) / "Git/bin/bash.exe"
        if candidate.is_file():
            return str(candidate)
    return shutil.which("bash")


class CodexHookTests(unittest.TestCase):
    def test_lifecycle_hooks_finish_and_keep_status_and_mail(self):
        bash = bash_path()
        self.assertIsNotNone(bash, "these tests require bash (Git Bash on Windows)")
        with tempfile.TemporaryDirectory(prefix="ngterm-hook-") as directory:
            root = Path(directory)
            recorder = root / "recorder.sh"
            recorder.write_text(
                '#!/usr/bin/env bash\n'
                'printf "%s\\n" "$*" >> "$HOOK_CALLS"\n'
                'if [ "$1 $2" = "agent mail" ]; then printf "%s\\n" "{\\"additionalContext\\":\\"mail\\"}"; fi\n',
                encoding="utf-8",
            )
            recorder.chmod(0o755)
            env = {key: value for key, value in os.environ.items() if not key.startswith("NGTERM_")}
            env.update(
                NGTERM_SESSION_ID="isolated-hook-test",
                NGTERM_PANE_ID="isolated-pane",
                NGTERM_SOCKET=(root / "unused.sock").as_posix(),
                NGTERMCTL=recorder.as_posix(),
                NGTERM_STATUS_WRAPPER=recorder.as_posix(),
                NGTERM_CODEX_WATCH_FILE=(root / "watch-token").as_posix(),
                NGTERM_CODEX_WATCH_MAX_CHECKS="1",
                NGTERM_CODEX_WATCH_INTERVAL="0",
                HOOK_CALLS=(root / "calls").as_posix(),
            )
            for event in ("session-start", "user-prompt-submit", "pre-tool-use", "post-tool-use", "permission-request"):
                with self.subTest(event=event):
                    (root / "calls").write_text("", encoding="utf-8")
                    result = subprocess.run(
                        [bash, (ROOT / "resources/agent-status/ngterm-codex-status.sh").as_posix(), event],
                        input='{"transcript_path":"rollout.jsonl"}',
                        capture_output=True, text=True, env=env, timeout=5,
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)
                    calls = (root / "calls").read_text(encoding="utf-8")
                    if event in ("user-prompt-submit", "pre-tool-use", "post-tool-use"):
                        self.assertIn("active --blink", calls)
                    if event in ("user-prompt-submit", "post-tool-use"):
                        self.assertIn('"additionalContext":"mail"', result.stdout)
                    if event == "permission-request":
                        self.assertIn("agent hook codex PermissionRequest", calls)
                    if os.name == "nt":
                        self.assertFalse((root / "watch-token").exists(), "a Windows hook must not start a persistent watcher")


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]] if sys.argv[1:] == ["--self-test"] else sys.argv)
