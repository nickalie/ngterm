//! Upstream's `DuplicateReplayTests.swift`, case by case, on a resolved-file seam; then the Windows lookup.

use std::collections::HashMap;

use super::*;
use crate::restart_replay::Refusal as Replay;

const HTOP: &str = "/opt/homebrew/bin/htop";

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

fn program(parts: &[&str]) -> PaneForeground {
    PaneForeground::Program(strings(parts))
}

/// One plan's inputs, upstream's `plan(...)` defaults.
struct Case {
    foreground: Option<PaneForeground>,
    leader: bool,
    cwd: Option<&'static str>,
    executable: Option<&'static str>,
    search_path: Option<&'static str>,
    files: Vec<(&'static str, &'static str)>,
    denylist: Vec<String>,
    lookup: Lookup,
}

impl Case {
    fn new(foreground: Option<PaneForeground>) -> Self {
        Self {
            foreground,
            leader: false,
            cwd: Some("/work"),
            executable: Some(HTOP),
            search_path: Some("/opt/homebrew/bin"),
            files: vec![(HTOP, HTOP)],
            denylist: Vec::new(),
            lookup: Lookup::posix(),
        }
    }

    fn cwd(mut self, cwd: Option<&'static str>) -> Self {
        self.cwd = cwd;
        self
    }

    fn executable(mut self, executable: Option<&'static str>) -> Self {
        self.executable = executable;
        self
    }

    fn search_path(mut self, search_path: Option<&'static str>) -> Self {
        self.search_path = search_path;
        self
    }

    fn files(mut self, files: &[(&'static str, &'static str)]) -> Self {
        self.files = files.to_vec();
        self
    }

    fn denylist(mut self, denylist: &[&str]) -> Self {
        self.denylist = strings(denylist);
        self
    }

    fn plan(&self) -> Result<Plan, Refusal> {
        let observation = Observation {
            foreground: self.foreground.clone(),
            is_daemon_leader: self.leader,
            working_directory: self.cwd.map(str::to_owned),
            executable_path: self.executable.map(str::to_owned),
            search_path: self.search_path.map(str::to_owned),
        };
        let files: HashMap<&str, &str> = self.files.iter().copied().collect();
        let resolved = |path: &str| files.get(path).map(|target| (*target).to_owned());
        let planner = Planner {
            shell: Some("zsh"),
            denylist: &self.denylist,
            lookup: &self.lookup,
            resolved_file: &resolved,
        };
        planner.resolve(&observation, "left", "/fallback")
    }
}

fn copied(argv: &[&str]) -> Result<Plan, Refusal> {
    copied_in(argv, "/work")
}

fn refusal(reason: Reason) -> Result<Plan, Refusal> {
    Err(Refusal {
        pane: "left".to_owned(),
        reason,
    })
}

#[test]
fn an_idle_shell_gives_a_plain_shell_where_it_sits() {
    let shell = Some(PaneForeground::ForegroundShell("zsh".to_owned()));
    assert_eq!(
        Case::new(shell.clone()).plan(),
        Ok(Plan::Shell {
            working_directory: "/work".to_owned()
        })
    );
    assert_eq!(
        Case::new(shell).cwd(None).plan(),
        Ok(Plan::Shell {
            working_directory: "/fallback".to_owned()
        })
    );
}

#[test]
fn a_running_program_starts_again_from_where_it_was_found_in_its_directory() {
    assert_eq!(
        Case::new(Some(program(&["htop", "-d", "5"]))).plan(),
        copied(&[HTOP, "-d", "5"])
    );
    assert_eq!(
        Case::new(Some(program(&["/usr/bin/top", "-o", "cpu"])))
            .executable(None)
            .search_path(None)
            .plan(),
        copied(&["/usr/bin/top", "-o", "cpu"])
    );
}

/// The resolved image lost the invoked name, so vimdiff ran as plain vim and a venv python as the base interpreter.
#[test]
fn a_symlinked_name_is_copied_by_the_path_it_was_started_from() {
    let vimdiff = Case::new(Some(program(&["vimdiff", "a", "b"])))
        .executable(Some("/usr/bin/vim"))
        .search_path(Some("/opt/homebrew/bin:/usr/bin"))
        .files(&[("/usr/bin/vimdiff", "/usr/bin/vim"), ("/usr/bin/vim", "/usr/bin/vim")]);
    assert_eq!(vimdiff.plan(), copied(&["/usr/bin/vimdiff", "a", "b"]));
    let python = "/usr/local/bin/python3.12";
    let venv = Case::new(Some(program(&["python", "app.py"])))
        .executable(Some(python))
        .search_path(Some("/work/.venv/bin:/usr/local/bin"))
        .files(&[
            ("/work/.venv/bin/python", python),
            ("/usr/local/bin/python", python),
            (python, python),
        ]);
    assert_eq!(venv.plan(), copied(&["/work/.venv/bin/python", "app.py"]));
}

#[test]
fn a_path_entry_holding_another_program_of_that_name_is_passed_over() {
    let other = "/usr/local/bin/htop";
    let case = Case::new(Some(program(&["htop"])))
        .search_path(Some("/usr/local/bin:/opt/homebrew/bin"))
        .files(&[(other, other), (HTOP, HTOP)]);
    assert_eq!(case.plan(), copied(&[HTOP]));
}

#[test]
fn a_relative_path_is_copied_from_the_programs_directory() {
    let python = "/usr/local/bin/python3.12";
    let venv = Case::new(Some(program(&["./venv/bin/python", "x"])))
        .executable(Some(python))
        .files(&[("/work/venv/bin/python", python), (python, python)]);
    assert_eq!(venv.plan(), copied(&["/work/venv/bin/python", "x"]));
    let up = Case::new(Some(program(&["../bin/tool"])))
        .executable(Some("/bin/tool"))
        .files(&[("/bin/tool", "/bin/tool")]);
    assert_eq!(up.plan(), copied(&["/bin/tool"]));
    let missing = Case::new(Some(program(&["./missing"]))).files(&[(HTOP, HTOP)]);
    assert_eq!(missing.plan(), refusal(Reason::NotLocated));
    let nowhere = Case::new(Some(program(&["./tool"]))).cwd(None);
    assert_eq!(nowhere.plan(), refusal(Reason::Replay(Replay::NoDirectory)));
}

/// A version manager's shim re-execs the real binary under its bare name, so no `PATH` entry resolves to the image.
#[test]
fn with_no_entry_resolving_to_the_image_the_shells_own_pick_is_copied() {
    let ruby = "/Users/u/.rbenv/versions/3.3.0/bin/ruby";
    let shim = "/Users/u/.rbenv/shims/ruby";
    let case = Case::new(Some(program(&["ruby", "app.rb"])))
        .executable(Some(ruby))
        .search_path(Some("/Users/u/.rbenv/shims:/usr/bin"))
        .files(&[(shim, shim), ("/usr/bin/ruby", "/usr/bin/ruby"), (ruby, ruby)]);
    assert_eq!(case.plan(), copied(&[shim, "app.rb"]));
    let relative = Case::new(Some(program(&["./tool"])))
        .executable(Some("/opt/x/tool"))
        .files(&[("/work/tool", "/work/tool"), ("/opt/x/tool", "/opt/x/tool")]);
    assert_eq!(relative.plan(), copied(&["/work/tool"]));
}

/// macOS withholds a platform binary's environment, so every bare /usr/bin program read no `PATH` and was refused.
#[test]
fn with_no_readable_path_the_system_directories_are_searched_then_the_image_is_copied() {
    let tee = Case::new(Some(program(&["tee", "out.txt"])))
        .executable(Some("/usr/bin/tee"))
        .search_path(None)
        .files(&[("/usr/bin/tee", "/usr/bin/tee")]);
    assert_eq!(tee.plan(), copied(&["/usr/bin/tee", "out.txt"]));
    let vimdiff = Case::new(Some(program(&["vimdiff", "a"])))
        .executable(Some("/usr/bin/vim"))
        .search_path(None)
        .files(&[("/usr/bin/vimdiff", "/usr/bin/vim"), ("/usr/bin/vim", "/usr/bin/vim")]);
    assert_eq!(vimdiff.plan(), copied(&["/usr/bin/vimdiff", "a"]));
    assert_eq!(
        Case::new(Some(program(&["htop"]))).search_path(None).plan(),
        copied(&[HTOP])
    );
}

#[test]
fn a_program_not_found_where_it_was_started_is_refused_rather_than_copied_as_the_image() {
    for search_path in ["", "bin:/nowhere"] {
        let case = Case::new(Some(program(&["htop"])))
            .search_path(Some(search_path))
            .files(&[("bin/htop", HTOP), (HTOP, HTOP)]);
        assert_eq!(case.plan(), refusal(Reason::NotLocated), "{search_path}");
    }
}

/// A package upgrade removed a running codex's image, so every copy of its pane was refused.
#[test]
fn a_bare_program_whose_image_is_gone_starts_what_its_path_names_now() {
    let codex = "/opt/homebrew/bin/codex";
    let case = Case::new(Some(program(&["codex", "-c", "x"])))
        .executable(None)
        .search_path(Some("/nowhere:relative:/opt/homebrew/bin"))
        .files(&[(codex, "/opt/homebrew/Caskroom/codex/2/bin/codex")]);
    assert_eq!(case.plan(), copied(&[codex, "-c", "x"]));
}

#[test]
fn a_program_whose_image_is_gone_with_nothing_on_its_path_is_refused() {
    let cases: [(&str, Option<&'static str>); 4] = [
        ("htop", Some("/nowhere")),
        ("htop", None),
        ("htop", Some("")),
        ("./tool", Some("/opt/homebrew/bin")),
    ];
    for (name, search_path) in cases {
        let case = Case::new(Some(program(&[name])))
            .executable(None)
            .search_path(search_path)
            .files(&[(HTOP, HTOP), ("/work/tool", "/work/tool")]);
        assert_eq!(case.plan(), refusal(Reason::NoExecutable), "{name} {search_path:?}");
    }
}

#[test]
fn a_program_whose_image_is_gone_still_passes_the_replay_checks() {
    let codex = "/opt/homebrew/bin/codex";
    let files = [(codex, "/opt/homebrew/Caskroom/codex/2/bin/codex")];
    let gone = |argv: &[&str]| Case::new(Some(program(argv))).executable(None).files(&files);
    assert_eq!(
        gone(&["codex"]).cwd(None).plan(),
        refusal(Reason::Replay(Replay::NoDirectory))
    );
    assert_eq!(gone(&["codex", r"a\b"]).plan(), refusal(Reason::Backslash));
    assert_eq!(
        gone(&["codex"]).denylist(&["codex"]).plan(),
        refusal(Reason::Replay(Replay::Denylisted("codex".to_owned())))
    );
    let vi = Case::new(Some(program(&["vi"])))
        .executable(None)
        .files(&[("/opt/homebrew/bin/vi", "/usr/bin/vim")])
        .denylist(&["vim"]);
    assert_eq!(vi.plan(), refusal(Reason::Replay(Replay::Denylisted("vim".to_owned()))));
}

#[test]
fn a_daemon_leader_shell_running_something_is_refused() {
    let mut case = Case::new(Some(program(&["/bin/zsh", "script.sh"])));
    case.leader = true;
    assert_eq!(case.plan(), refusal(Reason::Replay(Replay::Shell("zsh".to_owned()))));
}

#[test]
fn an_argument_with_a_backslash_is_refused() {
    for argv in [["grep", r"a\.b"], ["printf", r"x\"]] {
        let path: &'static str = Box::leak(format!("/usr/bin/{}", argv[0]).into_boxed_str());
        let case = Case::new(Some(program(&argv)))
            .executable(Some(path))
            .search_path(Some("/usr/bin"))
            .files(&[(path, path)]);
        assert_eq!(case.plan(), refusal(Reason::Backslash), "{argv:?}");
    }
}

#[test]
fn the_checks_see_both_the_path_and_the_image() {
    let odd = Case::new(Some(program(&["htop"])))
        .executable(Some(r"/odd\dir/htop"))
        .search_path(Some(r"/odd\dir"))
        .files(&[(r"/odd\dir/htop", r"/odd\dir/htop")]);
    assert_eq!(odd.plan(), refusal(Reason::Backslash));
    let vim = [("/usr/bin/vi", "/usr/bin/vim"), ("/usr/bin/vim", "/usr/bin/vim")];
    let vi = Case::new(Some(program(&["vi"])))
        .executable(Some("/usr/bin/vim"))
        .search_path(Some("/usr/bin"))
        .files(&vim)
        .denylist(&["vim"]);
    assert_eq!(vi.plan(), refusal(Reason::Replay(Replay::Denylisted("vim".to_owned()))));
    let vimdiff = Case::new(Some(program(&["vimdiff"])))
        .executable(Some("/usr/bin/vim"))
        .search_path(Some("/usr/bin"))
        .files(&[("/usr/bin/vimdiff", "/usr/bin/vim"), ("/usr/bin/vim", "/usr/bin/vim")])
        .denylist(&["vimdiff"]);
    assert_eq!(
        vimdiff.plan(),
        refusal(Reason::Replay(Replay::Denylisted("vimdiff".to_owned())))
    );
}

#[test]
fn a_refused_pane_is_named_and_nothing_is_created() {
    let cases = [
        (
            Some(program(&["/usr/bin/vim"])),
            vec!["vim"],
            Replay::Denylisted("vim".to_owned()),
        ),
        (None, Vec::new(), Replay::Unreadable),
    ];
    for (foreground, denylist, reason) in cases {
        let expected = Refusal {
            pane: "left".to_owned(),
            reason: Reason::Replay(reason.clone()),
        };
        assert_eq!(Case::new(foreground).denylist(&denylist).plan(), Err(expected.clone()));
        assert_eq!(
            expected.message(),
            format!("left pane: {}; nothing was created", reason.reason())
        );
        assert!(!expected.message().contains("--command"));
    }
}

#[test]
fn a_program_without_a_directory_is_refused() {
    let make = "/usr/bin/make";
    let case = Case::new(Some(program(&["make"])))
        .cwd(None)
        .executable(Some(make))
        .search_path(Some("/usr/bin"))
        .files(&[(make, make)]);
    assert_eq!(case.plan(), refusal(Reason::Replay(Replay::NoDirectory)));
}

#[test]
fn the_other_refusals_name_their_reason() {
    let message = |reason| {
        Refusal {
            pane: "right".to_owned(),
            reason,
        }
        .message()
    };
    assert_eq!(
        message(Reason::Backslash),
        "right pane: the foreground program's arguments contain a backslash, which cannot be copied; nothing was \
         created"
    );
    assert_eq!(
        message(Reason::NoExecutable),
        "right pane: the foreground program's executable path cannot be read; nothing was created"
    );
    assert_eq!(
        message(Reason::NotLocated),
        "right pane: the path the foreground program was started from cannot be found; nothing was created"
    );
}

fn windows(case: Case) -> Case {
    Case {
        lookup: Lookup::windows(r"C:\Windows", ".COM;.EXE;.BAT;.CMD"),
        ..case
    }
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "compared with a planner's answer, which is a Result"
)]
fn copied_in(argv: &[&str], directory: &str) -> Result<Plan, Refusal> {
    Ok(Plan::Program {
        argv: strings(argv),
        working_directory: directory.to_owned(),
    })
}

/// A bare name is looked up in each drive- or UNC-rooted `;` entry with each `PATHEXT` extension, names compared in
/// any case, and a backslash is no refusal.
#[test]
fn a_windows_name_is_found_with_its_extension_in_any_case() {
    let node = r"C:\Program Files\nodejs\node.exe";
    let case = windows(Case::new(Some(program(&["node", r"C:\work\app.js"]))))
        .cwd(Some(r"C:\work"))
        .executable(Some(node))
        .search_path(Some(r#"relative;"C:\Program Files\nodejs";C:\tools"#))
        .files(&[
            (r"C:\Program Files\nodejs\node.EXE", node),
            (node, r"c:\program files\nodejs\NODE.exe"),
        ]);
    assert_eq!(
        case.plan(),
        copied_in(&[r"C:\Program Files\nodejs\node.EXE", r"C:\work\app.js"], r"C:\work")
    );
    let git = r"\\server\share\bin\git.exe";
    let named = windows(Case::new(Some(program(&["git.exe", "log"]))))
        .cwd(Some(r"C:\work"))
        .executable(Some(git))
        .search_path(Some(r"\\server\share\bin"))
        .files(&[(git, git)]);
    assert_eq!(named.plan(), copied_in(&[git, "log"], r"C:\work"));
}

#[test]
fn a_windows_relative_name_and_the_system_directories() {
    let tool = r"C:\work\bin\tool.CMD";
    let relative = windows(Case::new(Some(program(&[r".\sub\..\bin\tool"]))))
        .cwd(Some(r"C:\work"))
        .executable(Some(r"C:\Windows\System32\cmd.exe"))
        .files(&[(tool, tool)]);
    assert_eq!(relative.plan(), copied_in(&[tool], r"C:\work"));
    let whoami = r"C:\Windows\System32\whoami.exe";
    let system = windows(Case::new(Some(program(&["whoami"]))))
        .cwd(Some(r"C:\work"))
        .executable(Some(whoami))
        .search_path(None)
        .files(&[(whoami, whoami)]);
    assert_eq!(system.plan(), copied_in(&[whoami], r"C:\work"));
    let absolute = windows(Case::new(Some(program(&[r"D:\bin\x.exe"])))).cwd(Some(r"D:\"));
    assert_eq!(absolute.plan(), copied_in(&[r"D:\bin\x.exe"], r"D:\"));
}

#[test]
fn a_windows_path_joins_under_its_root() {
    assert_eq!(PathRules::Windows.joined(r"..\x\y.exe", r"C:\a\b"), r"C:\a\x\y.exe");
    assert_eq!(PathRules::Windows.joined(r".\y", r"\\srv\share\d"), r"\\srv\share\d\y");
    assert_eq!(
        PathRules::Windows.joined(r"\tools\x.exe", r"C:\work"),
        r"C:\tools\x.exe"
    );
    assert_eq!(
        PathRules::Windows.joined("/tools/x.exe", r"\\srv\share\d"),
        r"\\srv\share\tools\x.exe"
    );
    assert_eq!(PathRules::Posix.joined("../../../x", "/a"), "/x");
    assert_eq!(without_verbatim_prefix(r"\\?\C:\a".to_owned()), r"C:\a");
    assert_eq!(without_verbatim_prefix(r"\\?\UNC\srv\s\a".to_owned()), r"\\srv\s\a");
    assert_eq!(without_verbatim_prefix("/usr/bin".to_owned()), "/usr/bin");
}

#[cfg(unix)]
#[test]
fn a_canonical_executable_resolves_symlinks_and_needs_the_executable_bit() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::write(&real, "#!/bin/sh\n").unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert_eq!(canonical_executable(link.to_str().unwrap()), None, "not executable");
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
    let canonical = std::fs::canonicalize(&real).unwrap();
    assert_eq!(
        canonical_executable(link.to_str().unwrap()).as_deref(),
        canonical.to_str()
    );
    assert_eq!(canonical_executable(dir.path().to_str().unwrap()), None, "a directory");
    assert_eq!(canonical_executable("/no/such/file"), None);
}
