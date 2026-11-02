"""Owned launcher stand-in with a child bus, both reaped on shutdown."""
import os
import signal
import subprocess
import sys
from pathlib import Path

child = subprocess.Popen([sys.executable, '-c', 'import signal; signal.pause()'])
Path(os.environ['XDG_RUNTIME_DIR'], 'child.pid').write_text(str(child.pid))
def stop(*args):
    child.wait(timeout=5)
    raise SystemExit(0)
signal.signal(signal.SIGTERM, stop)
signal.pause()
