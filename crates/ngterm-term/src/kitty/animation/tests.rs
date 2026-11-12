use super::*;

const ANIMATION: &[u8] = concat!(
    "\x1b_Ga=T,q=2,f=32,i=1,s=1,v=1,c=1,r=1;/wAA/w==\x1b\\",
    "\x1b_Ga=f,q=2,f=32,i=1,s=1,v=1,z=40;AAD//w==\x1b\\",
    "\x1b_Ga=a,q=2,i=1,r=1,z=30\x1b\\",
    "\x1b_Ga=a,q=2,i=1,s=3\x1b\\",
)
.as_bytes();

#[test]
fn animation_deadlines_are_absolute_and_unrelated_paints_cannot_postpone_them() {
    let start = Instant::now();
    let mut animation = Animation::new(start);
    assert_eq!(
        animation.schedule(start, Some(30)),
        Some(start + Duration::from_millis(30))
    );
    for ms in [10, 20, 29, 31] {
        assert_eq!(
            animation.schedule(start + Duration::from_millis(ms), Some(30)),
            Some(start + Duration::from_millis(30))
        );
    }
    assert_eq!(animation.schedule(start, Some(0)), Some(start + DRAW_INTERVAL));
    assert_eq!(animation.schedule(start, None), None);
    assert_eq!(animation.next, None);
}

#[test]
fn animation_clock_ticks_live_frames_and_preserves_held_pixels() {
    crate::init();
    let mut terminal = vt::Terminal::new(20, 5).unwrap();
    terminal.set_kitty_image_storage_limit(1_000_000).unwrap();
    terminal.vt_write(ANIMATION);
    let start = Instant::now();
    let mut animation = Animation::new(start);
    assert_eq!(
        animation.tick(&mut terminal, start, false).unwrap(),
        Some(start + Duration::from_millis(30))
    );
    let generation = terminal.kitty_graphics().unwrap().image(1).unwrap().generation;
    assert_eq!(
        animation
            .tick(&mut terminal, start + Duration::from_secs(1), true)
            .unwrap(),
        None
    );
    {
        let graphics = terminal.kitty_graphics().unwrap();
        let image = graphics.image(1).unwrap();
        assert_eq!(image.generation, generation);
        assert_eq!(image.pixels.unwrap(), [255, 0, 0, 255]);
    }
    assert_eq!(
        animation
            .tick(&mut terminal, start + Duration::from_secs(1), false)
            .unwrap(),
        Some(start + Duration::from_millis(1040))
    );
    assert_eq!(
        terminal.kitty_graphics().unwrap().image(1).unwrap().pixels.unwrap(),
        [0, 0, 255, 255]
    );
    terminal.set_kitty_image_storage_limit(0).unwrap();
    assert_eq!(
        animation
            .tick(&mut terminal, start + Duration::from_secs(2), false)
            .unwrap(),
        None
    );
}
