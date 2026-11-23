import pytest
from agterm_conformance.match import assert_ok, assert_version_result
from agterm_conformance.plugin import Ctl


@pytest.mark.covers("version", "field:result.app")
def test_version_reports_the_app_version(ctl):
    assert_version_result(ctl.request("version"))


@pytest.mark.covers("version", "field:result.app", "field:tree.app")
def test_tree_app_is_the_version_identity(ctl):
    assert ctl.tree()["tree"]["app"] == assert_ok(ctl.request("version"))["app"]


@pytest.mark.covers("version", "field:result.installed")
def test_version_reports_the_installed_build_when_it_is_known(ctl):
    # The build under test is the one installed: the same version and commit as the running app. Not on the tree.
    result = assert_version_result(ctl.request("version"))
    assert result.get("installed") == result["app"], result
    assert "installed" not in ctl.tree()["tree"]["app"]


@pytest.mark.covers("version", "field:result.installed")
def test_installed_follows_the_file_on_disk(launch, adapter):
    # A private install, so rewriting its metadata after launch never touches the build under test.
    ctl = launch(private_install=True)
    app = assert_version_result(ctl.request("version"))["app"]

    adapter.write_installed(ctl.instance, "99.0.0", "abc1234")
    replaced = assert_version_result(ctl.request("version"))
    assert replaced["installed"] == {"version": "99.0.0", "commit": "abc1234"}, replaced
    assert replaced["app"] == app

    adapter.write_installed(ctl.instance, "99.0.1", "unknown")
    assert assert_version_result(ctl.request("version"))["installed"] == {"version": "99.0.1"}

    adapter.write_installed(ctl.instance, None)
    removed = assert_version_result(ctl.request("version"))
    assert "installed" not in removed, removed
    assert removed["app"] == app



@pytest.mark.since("0.42.0")
@pytest.mark.covers("version", "field:result.headless", "field:result.versions")
def test_an_app_with_windows_reports_neither_headless_nor_versions(ctl):
    result = assert_version_result(ctl.request("version"))
    assert "headless" not in result and "versions" not in result, result


@pytest.mark.since("0.42.0")
@pytest.mark.requires("posix-command-line", reason="the remote origin's sshd is POSIX only")
@pytest.mark.requires("live-any-shell", reason="ngterm-headless runs every session Live, in any login shell")
@pytest.mark.parametrize("remote_origin", ["headless"], indirect=True)
@pytest.mark.covers("version", "field:result.headless", "field:result.versions")
def test_a_headless_origin_reports_headless_and_what_it_serves(remote_origin):
    """`ngterm-headless` (upstream's agterm-server) reports `headless`, where a pane's own untargeted pick belongs to its
    session, and the versions of its build, the presentation stream and the catalog stream."""
    result = assert_ok(Ctl(remote_origin.instance).request("version"))
    assert result["headless"] is True, result
    assert result["versions"] == {"server": result["app"]["version"], "protocol": 1, "catalog": 1}, result
