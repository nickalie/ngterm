use super::*;

#[test]
fn kitty_animation_capture_never_ticks_a_synchronized_output_hold() {
    crate::init();
    let (events, _) = mpsc::channel();
    let mailbox = Arc::new(Mailbox::new(ReadPause::new()));
    let config = PaneConfig::new(GridSize {
        cols: 20,
        rows: 5,
        cell_width: 10,
        cell_height: 20,
    });
    let shared = shared(&config, events, mailbox).unwrap();
    {
        let mut vt = lock(&shared.vt);
        vt.terminal.vt_write(
            concat!(
                "\x1b_Ga=T,q=2,f=32,i=1,s=1,v=1,c=1,r=1;/wAA/w==\x1b\\",
                "\x1b_Ga=f,q=2,f=32,i=1,s=1,v=1,z=40;AAD//w==\x1b\\",
                "\x1b_Ga=a,q=2,i=1,r=1,z=30\x1b\\",
                "\x1b_Ga=a,q=2,i=1,s=3\x1b\\",
            )
            .as_bytes(),
        );
        assert_eq!(
            vt.terminal.kitty_graphics().unwrap().animation_tick(0).unwrap(),
            Some(30)
        );
        // An expired animation on a deterministic clock, while the hold itself has only just begun.
        vt.kitty_animation = crate::kitty::Animation::new(Instant::now().checked_sub(Duration::from_secs(1)).unwrap());
        vt.terminal.vt_write(b"\x1b[?2026h");
    }
    let held = capture(&shared).unwrap();
    assert!(held.held);
    let red = &held.kitty.images[0];
    assert_eq!(&*red.pixels, &[255, 0, 0, 255]);
    let unchanged = capture(&shared).unwrap();
    assert!(Arc::ptr_eq(&held, &unchanged));
    lock(&shared.vt).terminal.vt_write(b"\x1b[?2026l");
    let live = capture(&shared).unwrap();
    assert!(!live.held);
    assert_eq!(&*live.kitty.images[0].pixels, &[0, 0, 255, 255]);
    assert!(live.kitty.images[0].key.generation > red.key.generation);
    assert_eq!(&*red.pixels, &[255, 0, 0, 255]);
}
