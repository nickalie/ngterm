import pytest
from agterm_conformance.match import assert_ok, assert_version_result


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
