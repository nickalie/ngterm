use super::*;

#[test]
fn a_failed_signing_or_zip_reports_the_tool_s_own_error() {
    let fixture = tempfile::tempdir().unwrap();
    let missing = fixture.path().join(ROOT);
    let signed = seal(&missing).unwrap_err();
    assert_eq!(signed.kind(), std::io::ErrorKind::Other);
    assert!(
        signed.to_string().contains("ngterm.app"),
        "codesign's error names the bundle: {signed}"
    );
    let zipped = zip(&missing).unwrap_err();
    assert_eq!(zipped.kind(), std::io::ErrorKind::Other);
    assert!(!zipped.to_string().is_empty(), "ditto's standard error is reported");
}
