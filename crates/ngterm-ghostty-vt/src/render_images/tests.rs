use base64::Engine as _;

use crate::{Error, KittyLayer, RenderImage, RenderState, Terminal};

fn terminal() -> Terminal {
    let mut terminal = Terminal::new(10, 5).unwrap();
    terminal.resize(10, 5, 10, 10).unwrap();
    terminal.set_kitty_image_storage_limit(10_000_000).unwrap();
    terminal
}

fn images(state: &mut RenderState) -> Vec<RenderImage> {
    let mut walk = state.images().unwrap();
    let mut result = Vec::new();
    while let Some(image) = walk.next_image().unwrap() {
        result.push(image);
    }
    assert_eq!(walk.next_image().unwrap(), None, "exhausted");
    result
}

fn upload(terminal: &mut Terminal, id: u32, size: (u32, u32), keys: &str) {
    let bytes = vec![255u8; usize::try_from(size.0 * size.1 * 3).unwrap()];
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    let chunks: Vec<_> = data.as_bytes().chunks(4096).collect();
    for (index, chunk) in chunks.iter().enumerate() {
        let more = u8::from(index + 1 < chunks.len());
        let header = if index == 0 {
            format!("a=t,f=24,i={id},s={},v={},q=2{keys},m={more}", size.0, size.1)
        } else {
            format!("m={more}")
        };
        terminal.vt_write(format!("\x1b_G{header};{}\x1b\\", std::str::from_utf8(chunk).unwrap()).as_bytes());
    }
}

#[test]
fn kitty_empty_exhausted_and_failed_captures_protect_the_iterator() {
    let mut terminal = terminal();
    let mut state = RenderState::new().unwrap();
    assert!(images(&mut state).is_empty());
    state.update(&mut terminal).unwrap();
    assert!(images(&mut state).is_empty());
    std::mem::forget(state.begin_update(&mut terminal).unwrap());
    assert_eq!(state.images().err(), Some(Error::NoValue));
    assert_eq!(state.image_placements_generation(), None);
    state.update(&mut terminal).unwrap();
    assert!(images(&mut state).is_empty());
}

#[test]
fn kitty_virtual_fixture_has_resolved_geometry_and_matching_pixel_generation() {
    let mut terminal = terminal();
    terminal.vt_write(b"\x1b_Ga=T,t=d,f=24,i=1,p=1,s=1,v=2,U=1,c=2,r=1,q=2;////////\x1b\\");
    terminal.vt_write("\x1b[2;4H\x1b[38;5;1;58;5;1m\u{10EEEE}\u{10EEEE}".as_bytes());
    let mut state = RenderState::new().unwrap();
    let mut pending = state.begin_update(&mut terminal).unwrap();
    let image = pending.images().unwrap().next_image().unwrap().unwrap();
    assert_eq!((image.image_id, image.placement_id), (1, 1));
    assert_eq!((image.column, image.row, image.columns, image.rows), (3, 1, 2, 1));
    assert_eq!((image.width, image.height, image.offset_x), (5, 10, 8));
    assert_eq!((image.z, image.layer), (-1, KittyLayer::BelowText));
    assert_eq!(
        image.image_generation,
        terminal.kitty_graphics().unwrap().image(1).unwrap().generation
    );
    pending.end().unwrap();
    assert_eq!(images(&mut state), [image]);
}

#[test]
fn kitty_fixed_fixture_preserves_offsets_source_and_negative_viewport_rows() {
    let mut terminal = terminal();
    upload(&mut terminal, 1, (1, 1), "");
    terminal.vt_write(b"\x1b[2;3H\x1b_Ga=p,i=1,p=7,c=3,r=2,X=4,Y=3,z=-5,C=1,q=2\x1b\\");
    let mut state = RenderState::new().unwrap();
    state.update(&mut terminal).unwrap();
    let image = images(&mut state)[0];
    assert_eq!((image.column, image.row, image.columns, image.rows), (2, 1, 3, 2));
    assert_eq!((image.offset_x, image.offset_y, image.z), (4, 3, -5));
    assert_eq!((image.source.width, image.source.height), (1, 1));
    terminal.vt_write(b"\x1b_Ga=d,d=a,q=2\x1b\\\x1b[1;2H\x1b_Ga=p,i=1,p=1,c=2,r=3,C=1,q=2\x1b\\\x1b[5;1H\n\n");
    state.update(&mut terminal).unwrap();
    let image = images(&mut state)[0];
    assert_eq!((image.column, image.row, image.width, image.height), (1, -2, 20, 30));
}

#[test]
fn kitty_dog_fit_cases_and_half_image_rows_come_from_the_api() {
    let mut terminal = terminal();
    terminal.resize(10, 5, 36, 80).unwrap();
    let mut state = RenderState::new().unwrap();
    for (cols, rows, width, height, offset) in [(4, 2, 144, 44, 36), (2, 2, 72, 22, 58), (1, 1, 36, 22, 29)] {
        terminal.vt_write(b"\x1b[2J\x1b[H");
        upload(&mut terminal, 1, (500, 306), "");
        terminal.vt_write(format!("\x1b_Ga=p,U=1,i=1,p=1,c={cols},r={rows},q=2\x1b\\\x1b[38;5;1;58;5;1m").as_bytes());
        terminal.vt_write("\u{10EEEE}\u{0305}\u{0305}\u{10EEEE}\u{10EEEE}\u{10EEEE}".as_bytes());
        if rows == 2 {
            terminal.vt_write("\x1b[2;1H\u{10EEEE}\u{030D}\u{0305}\u{10EEEE}\u{10EEEE}\u{10EEEE}".as_bytes());
        }
        state.update(&mut terminal).unwrap();
        let drawn = images(&mut state);
        assert_eq!(
            drawn.len(),
            usize::try_from(rows).unwrap(),
            "{cols}x{rows}: {:?}",
            terminal.kitty_graphics().unwrap().placements()
        );
        assert_eq!(
            (drawn[0].width, drawn[0].height, drawn[0].offset_y),
            (width, height, offset)
        );
        if rows == 2 {
            assert_eq!((drawn[0].source.y, drawn[0].source.height), (0, 153));
            assert_eq!((drawn[1].source.y, drawn[1].source.height), (153, 153));
        }
    }
    terminal.vt_write(b"\x1b[2J\x1b[H");
    upload(&mut terminal, 2, (10, 10), "");
    terminal.resize(10, 5, 10, 10).unwrap();
    terminal.vt_write(b"\x1b_Ga=p,U=1,i=2,p=1,c=2,r=2,q=2\x1b\\");
    terminal.vt_write(
        "\x1b[38;5;2;58;5;1m\u{10EEEE}\u{0305}\u{0305}\u{10EEEE}\x1b[2;1H\u{10EEEE}\u{030D}\u{0305}\u{10EEEE}"
            .as_bytes(),
    );
    state.update(&mut terminal).unwrap();
    let drawn = images(&mut state);
    assert_eq!(drawn.len(), 2);
    assert_eq!(
        (
            drawn[0].width,
            drawn[0].height,
            drawn[0].source.width,
            drawn[0].source.height
        ),
        (20, 10, 10, 5)
    );
    assert_eq!((drawn[0].source.y, drawn[1].source.y), (0, 5));
}

#[test]
fn kitty_placement_generation_changes_only_with_the_resolved_list() {
    let mut terminal = terminal();
    upload(&mut terminal, 1, (1, 1), "");
    let mut state = RenderState::new().unwrap();
    state.update(&mut terminal).unwrap();
    let empty = state.image_placements_generation();
    state.update(&mut terminal).unwrap();
    assert_eq!(state.image_placements_generation(), empty);
    terminal.vt_write(b"\x1b_Ga=p,i=1,p=1,c=2,r=2,C=1,q=2\x1b\\");
    state.update(&mut terminal).unwrap();
    let placed = state.image_placements_generation();
    assert_ne!(placed, empty);
    terminal.vt_write(b"\x1b[4;1Hhello");
    state.update(&mut terminal).unwrap();
    assert_eq!(state.image_placements_generation(), placed);
    terminal.vt_write(b"\x1b[5;1H\n");
    state.update(&mut terminal).unwrap();
    assert_ne!(state.image_placements_generation(), placed);
    let scrolled = state.image_placements_generation();
    terminal.vt_write(b"\x1b_Ga=d,d=a,q=2\x1b\\");
    state.update(&mut terminal).unwrap();
    assert_ne!(state.image_placements_generation(), scrolled);
    assert!(images(&mut state).is_empty());
}
