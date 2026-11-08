use super::raise_open_files;
use std::process::Command;

const PROBE: &str = "SESSION_TEST_DESCRIPTOR_LIMIT";

fn open_files_limit() -> libc::rlimit {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a writable structure of the size getrlimit expects.
    assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) }, 0);
    limit
}

#[test]
fn the_soft_limit_is_raised_within_the_hard_limit_and_never_lowered() {
    // A hard limit cannot be restored after lowering it. Each case gets its own process, with no concurrent tests.
    let probe = "limits::tests::descriptor_limit_probe";
    for case in ["low", "hard_cap", "at_cap", "already_high"] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([probe, "--exact", "--ignored", "--nocapture", "--test-threads=1"])
            .env(PROBE, case)
            .output()
            .unwrap();
        assert!(output.status.success(), "{case}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "{output:?}"
        );
    }
}

#[test]
#[ignore = "the descriptor-limit test's isolated child"]
fn descriptor_limit_probe() {
    let inherited = open_files_limit();
    let (soft, hard, expected) = match std::env::var(PROBE).unwrap().as_str() {
        "low" => (256, inherited.rlim_max, 8192),
        "hard_cap" => (256, 4096, 4096),
        "at_cap" => (512, 512, 512),
        "already_high" => (16384, inherited.rlim_max, 16384),
        case => panic!("unknown descriptor-limit case: {case}"),
    };
    let limit = libc::rlimit {
        rlim_cur: soft,
        rlim_max: hard,
    };
    // SAFETY: changes only this isolated child's limits, to values within its inherited hard limit.
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const limit) }, 0);
    raise_open_files();
    let actual = open_files_limit();
    assert_eq!(actual.rlim_cur, expected);
    assert_eq!(actual.rlim_max, hard);
    raise_open_files();
    let repeated = open_files_limit();
    assert_eq!(repeated.rlim_cur, expected, "raising the limit twice is harmless");
    assert_eq!(repeated.rlim_max, hard);
}
