"""The host probes of `command_line`."""

import sys

import pytest
from agterm_conformance.command_line import env_has_argv0

pytestmark = pytest.mark.skipif(sys.platform == "win32", reason="the probe runs a POSIX env")


def _env(tmp_path, body):
    path = tmp_path / "env"
    path.write_text(f"#!/bin/sh\n{body}\n")
    path.chmod(0o755)
    return str(path)


def test_an_env_that_takes_argv0_has_it(tmp_path):
    assert env_has_argv0(_env(tmp_path, '[ "$1" = --argv0 ] && [ "$2" = sh ] && shift 2 && exec "$@"'))


def test_an_env_that_refuses_argv0_lacks_it(tmp_path):
    assert not env_has_argv0(_env(tmp_path, "echo 'env: illegal option -- -' >&2; exit 1"))


def test_a_missing_env_lacks_argv0(tmp_path):
    assert not env_has_argv0(str(tmp_path / "no-env"))
