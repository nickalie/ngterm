"""The connect timeout of every ssh the viewer starts to a remote (ssh's `ConnectTimeout`): 10 s when `remotes.conf`
sets none, `connect-timeout = SECONDS` for every machine and `connect-timeout HOST = SECONDS` for one, a pinned
destination (`ngterm:HOST`) sharing its host's value. An edit reaches the next ssh; a bad value is logged as a
`remotes.conf` problem (the "issues in remotes.conf" banner's log form) and the ssh uses 10.

The setting is ngterm's only (upstream reads such a line as a remote); the reference claims no `live-any-shell`, so it
skips these tests. The viewer, the build's GUI or, in one test, the staged `ngterm-headless`, reaches the staged
`ngterm-headless` behind the harness's own sshd (`remote_origin`) through `ngtermctl live tree HOST`, and the tests read
the argv of each ssh it started in the private ssh client's call log (`SshClient.invocations()`)."""

import re

import pytest
from agterm_conformance.plugin import Cli
from agterm_conformance.remote_origin import ORIGIN_HOST, headless_program, launch_headless
from agterm_conformance.wait import wait_until

pytestmark = [
    pytest.mark.requires("posix-command-line", reason="the private sshd uses a POSIX shell"),
    pytest.mark.requires("live-any-shell", reason="the origin's sessions must be Live in any login shell"),
    pytest.mark.parametrize("remote_origin", ["headless"], indirect=True),
]

DEFAULT = 10
# the watcher reads an edited remotes.conf at once; this bounds a slow host
EDIT_SECONDS = 20.0


@pytest.fixture
def remotes_conf(remote_viewer):
    """The viewer's `remotes.conf`, removed after the test so the next one starts with none."""
    path = remote_viewer.instance.state_dir / "config" / "remotes.conf"
    path.parent.mkdir(exist_ok=True)
    path.unlink(missing_ok=True)
    yield path
    path.unlink(missing_ok=True)


@pytest.fixture
def vcli(adapter, remote_viewer) -> Cli:
    return Cli(adapter.ctl_path, remote_viewer.instance, adapter.ctl_name)


def _timeouts(argv):
    return [int(word.split("=", 1)[1]) for word in argv if word.startswith("ConnectTimeout=")]


def _list(vcli, remote_origin, host=ORIGIN_HOST):
    """`ngtermctl live tree HOST`, then the ConnectTimeout of the one ssh it started."""
    before = len(remote_origin.client.invocations())
    result = vcli("live", "tree", host)
    assert (result.returncode, result.stderr) == (0, ""), result.stdout
    started = remote_origin.client.invocations()[before:]
    assert started, "live tree started no ssh"
    timeouts = {seconds for argv in started for seconds in _timeouts(argv)}
    assert len(timeouts) == 1, started
    return timeouts.pop()


def _lists_with(vcli, remote_origin, seconds, host=ORIGIN_HOST):
    """Lists `host` until its ssh uses `seconds`, as the next ssh after the viewer reads the edited file."""
    def reached():
        found = _list(vcli, remote_origin, host)
        assert found == seconds, f"ConnectTimeout={found}, not {seconds}"
        return True

    wait_until(reached, timeout=EDIT_SECONDS)


def _problems(remote_viewer):
    text = remote_viewer.instance.log.read_text(errors="replace")
    return re.findall(r"remotes\.conf: (.*)", text)


@pytest.mark.covers("cli:live tree")
def test_an_ssh_waits_10_seconds_to_connect_when_remotes_conf_sets_nothing(vcli, remote_origin, remotes_conf):
    assert not remotes_conf.exists()
    assert _list(vcli, remote_origin) == DEFAULT
    assert _list(vcli, remote_origin, f"ngterm:{ORIGIN_HOST}") == DEFAULT


@pytest.mark.covers("cli:live tree")
def test_an_old_remotes_conf_without_the_setting_works_as_before(vcli, remote_viewer, remote_origin, remotes_conf):
    remotes_conf.write_text(f"connect-timeout = 25\n{ORIGIN_HOST} Origin\n")
    _lists_with(vcli, remote_origin, 25)
    problems = len(_problems(remote_viewer))

    remotes_conf.write_text(f"# my machines\n{ORIGIN_HOST} Origin\n")
    _lists_with(vcli, remote_origin, DEFAULT)
    result = vcli("live", "tree", ORIGIN_HOST)
    assert result.returncode == 0 and re.search(r"\[[0-9A-F-]{36}\]", result.stdout), (result.stdout, result.stderr)
    assert _problems(remote_viewer)[problems:] == []


@pytest.mark.covers("cli:live tree")
def test_a_file_wide_value_reaches_every_ssh_and_an_edit_reaches_the_next_one(vcli, remote_origin, remotes_conf):
    remotes_conf.write_text(f"connect-timeout = 25\n{ORIGIN_HOST} Origin\n")
    _lists_with(vcli, remote_origin, 25)
    assert _list(vcli, remote_origin, f"ngterm:{ORIGIN_HOST}") == 25

    remotes_conf.write_text(f"connect-timeout=7\n{ORIGIN_HOST} Origin\n")
    _lists_with(vcli, remote_origin, 7)

    remotes_conf.write_text(f"{ORIGIN_HOST} Origin\n")
    _lists_with(vcli, remote_origin, DEFAULT)


@pytest.mark.covers("cli:live tree")
def test_a_value_for_one_host_reaches_its_ssh_also_through_a_pin(vcli, remote_origin, remotes_conf):
    remotes_conf.write_text(
        f"connect-timeout = 25\nconnect-timeout ngterm:{ORIGIN_HOST} = 40\nconnect-timeout elsewhere = 3\n"
        f"{ORIGIN_HOST} Origin\n"
    )
    _lists_with(vcli, remote_origin, 40)
    assert _list(vcli, remote_origin, f"ngterm:{ORIGIN_HOST}") == 40

    remotes_conf.write_text(f"connect-timeout = 25\nconnect-timeout {ORIGIN_HOST} = 12\n{ORIGIN_HOST} Origin\n")
    _lists_with(vcli, remote_origin, 12, f"ngterm:{ORIGIN_HOST}")
    assert _list(vcli, remote_origin) == 12


@pytest.mark.covers("cli:live tree")
@pytest.mark.parametrize("value", ["abc", "0", "301", "1.5"])
def test_a_bad_value_is_a_remotes_conf_problem_and_the_ssh_uses_10(vcli, remote_viewer, remote_origin, remotes_conf,
                                                                   value):
    remotes_conf.write_text(f"connect-timeout = 25\n{ORIGIN_HOST} Origin\n")
    _lists_with(vcli, remote_origin, 25)
    problems = len(_problems(remote_viewer))

    remotes_conf.write_text(f"connect-timeout = {value}\n{ORIGIN_HOST} Origin\n")
    _lists_with(vcli, remote_origin, DEFAULT)
    logged = _problems(remote_viewer)[problems:]
    assert any(f"invalid connect-timeout '{value}'" in line for line in logged), logged
    result = vcli("live", "tree", ORIGIN_HOST)
    assert result.returncode == 0, (result.stdout, result.stderr)


@pytest.mark.covers("cli:live tree")
def test_a_bad_value_for_one_host_leaves_it_the_file_wide_value(vcli, remote_viewer, remote_origin, remotes_conf):
    problems = len(_problems(remote_viewer))
    remotes_conf.write_text(f"connect-timeout = 25\nconnect-timeout {ORIGIN_HOST} = soon\n{ORIGIN_HOST} Origin\n")
    _lists_with(vcli, remote_origin, 25)
    assert any("invalid connect-timeout 'soon'" in line for line in _problems(remote_viewer)[problems:])


@pytest.mark.covers("cli:live tree")
def test_a_value_in_remotes_conf_at_launch_reaches_the_first_ssh(adapter, launch, remote_origin):
    viewer = launch(path_prefix=remote_origin.viewer_path)
    config = viewer.instance.state_dir / "config"
    config.mkdir(exist_ok=True)
    (config / "remotes.conf").write_text(f"connect-timeout {ORIGIN_HOST} = 33\n{ORIGIN_HOST} Origin\n")
    viewer = launch.relaunch(viewer)
    assert _list(Cli(adapter.ctl_path, viewer.instance, adapter.ctl_name), remote_origin) == 33


@pytest.mark.covers("cli:live tree")
def test_a_headless_viewer_follows_edits_of_its_remotes_conf(adapter, remote_origin):
    program = headless_program(adapter)
    assert program is not None, "the headless origin runs, so the staged ngterm-headless exists"
    viewer = launch_headless(adapter, program, path_prefix=remote_origin.viewer_path)
    try:
        vcli = Cli(adapter.ctl_path, viewer, adapter.ctl_name)
        config = viewer.state_dir / "config"
        config.mkdir(exist_ok=True)
        (config / "remotes.conf").write_text(f"connect-timeout = 25\n{ORIGIN_HOST} Origin\n")
        _lists_with(vcli, remote_origin, 25)

        (config / "remotes.conf").write_text(f"connect-timeout = 7\n{ORIGIN_HOST} Origin\n")
        _lists_with(vcli, remote_origin, 7)
    finally:
        adapter.stop(viewer)
