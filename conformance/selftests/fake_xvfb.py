"""Xvfb stand-in: publishes a unique displayfd number and waits for termination."""
import os
import sys
import signal

fd = int(sys.argv[sys.argv.index('-displayfd') + 1])
os.write(fd, f'{os.getpid()}\n'.encode())
os.close(fd)
signal.pause()
