use super::RemoteLinkRefusal;

/// Upstream's `eachRefusalHasItsOwnText`: every reason travels by its raw value and reads back, and its text names the
/// session's host, with this build's name where upstream names its own.
#[test]
fn each_refusal_reads_back_and_names_the_host() {
    let expected = [
        (
            RemoteLinkRefusal::RevealIntent,
            "revealIntent",
            "the file is on box; only a file on this Mac can be revealed in Finder",
        ),
        (
            RemoteLinkRefusal::NotPresenter,
            "notPresenter",
            "another Mac presents this session, so box cannot show the file here",
        ),
        (
            RemoteLinkRefusal::OriginTooOld,
            "originTooOld",
            "the ngterm on box is too old to open clicked files",
        ),
        (
            RemoteLinkRefusal::PaneNotAttached,
            "paneNotAttached",
            "this pane is not attached to box",
        ),
        (
            RemoteLinkRefusal::Hidden,
            "hidden",
            "the page from box would be hidden here",
        ),
        (RemoteLinkRefusal::NotFound, "notFound", "no such file on box"),
        (
            RemoteLinkRefusal::NotAPage,
            "notAPage",
            "box cannot show this file as a page",
        ),
        (
            RemoteLinkRefusal::NotOpened,
            "notOpened",
            "box could not open the file here",
        ),
    ];
    assert_eq!(expected.len(), RemoteLinkRefusal::ALL.len());
    for (reason, raw, message) in expected {
        assert_eq!(reason.raw(), raw);
        assert_eq!(RemoteLinkRefusal::from_raw(raw), Some(reason));
        assert_eq!(reason.message("box"), message);
    }
    assert_eq!(RemoteLinkRefusal::from_raw("someday"), None);
}
