use super::*;
use crate::render::{Capture, Frame, MouseShape};

const CHUNKED: &[u8] = b"\x1b_Ga=T,f=32,s=2,v=1,i=7,m=1;AQIDBAUG\x1b\\\x1b_Gm=0;Bwg=\x1b\\";

fn terminal() -> vt::Terminal {
    crate::init();
    let mut terminal = vt::Terminal::new(20, 5).unwrap();
    terminal.resize(20, 5, 10, 20).unwrap();
    terminal.set_kitty_image_storage_limit(10_000_000).unwrap();
    terminal
}

fn frame(terminal: &mut vt::Terminal, state: &mut Capture, previous: &Frame, prefetch: &Prefetch) -> Option<Frame> {
    let (pending, kitty) = state.begin_update(terminal, prefetch).unwrap();
    pending.end().unwrap();
    state.finish_kitty(kitty);
    state.convert(previous, MouseShape::Default).unwrap()
}

fn image(terminal: &mut vt::Terminal, id: u32, keys: &str) {
    terminal.vt_write(format!("\x1b_Ga=t,q=2,f=24,s=1,v=1,i={id}{keys};AAAA\x1b\\").as_bytes());
}

fn at(view: &KittyView, id: u32) -> Vec<(i32, i32)> {
    view.placements
        .iter()
        .filter(|p| p.image.id == id)
        .map(|p| (p.rect.column, p.rect.row))
        .collect()
}

#[test]
fn kitty_pixels_are_shared_across_plain_updates_and_prefetch() {
    let mut terminal = terminal();
    terminal.vt_write(b"\x1b[2;1H");
    terminal.vt_write(CHUNKED);
    let mut prefetch = Prefetch::default();
    prefetch.update(terminal.kitty_graphics());
    let mut state = Capture::new().unwrap();
    let first = frame(&mut terminal, &mut state, &Frame::default(), &prefetch).unwrap();
    let image = &first.kitty.images[0];
    assert_eq!((image.key.id, image.width, image.height), (7, 2, 1));
    assert_eq!(&*image.pixels, &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert!(Arc::ptr_eq(image, prefetch.get(image.key).unwrap()));
    assert!(frame(&mut terminal, &mut state, &first, &prefetch).is_none());
    terminal.vt_write(b"\x1b[4;1Hhello");
    let next = frame(&mut terminal, &mut state, &first, &prefetch).unwrap();
    assert!(
        Arc::ptr_eq(&next.kitty, &first.kitty),
        "unchanged placement conversion is reused"
    );
    terminal.vt_write(b"\x1b[5;1H\n");
    let moved = frame(&mut terminal, &mut state, &next, &prefetch).unwrap();
    // Partly outside the viewport: no new pixels copied, full signed geometry retained.
    assert!(Arc::ptr_eq(&moved.kitty.images[0], image));
    assert_ne!(at(&moved.kitty, 7), at(&first.kitty, 7));
}

#[test]
fn kitty_placement_only_add_and_final_delete_publish_frames_with_clean_text_rows() {
    let mut terminal = terminal();
    image(&mut terminal, 1, "");
    let mut state = Capture::new().unwrap();
    let prefetch = Prefetch::default();
    let first = frame(&mut terminal, &mut state, &Frame::default(), &prefetch).unwrap();
    terminal.vt_write(b"\x1b_Ga=p,q=2,i=1,p=1,c=2,r=1,C=1\x1b\\");
    let second = frame(&mut terminal, &mut state, &first, &prefetch).unwrap();
    assert_eq!(second.kitty.placements.len(), 1);
    assert!(second.seq > first.seq);
    assert!(second.rows.iter().zip(&first.rows).all(|(a, b)| Arc::ptr_eq(a, b)));
    let key = second.kitty.images[0].key;
    terminal.vt_write(b"\x1b_Ga=d,q=2,d=a\x1b\\");
    let third = frame(&mut terminal, &mut state, &second, &prefetch).unwrap();
    assert!(third.seq > second.seq);
    assert!(third.kitty.placements.is_empty());
    assert_eq!(third.kitty.removed, [key]);
    assert!(third.rows.iter().zip(&second.rows).all(|(a, b)| Arc::ptr_eq(a, b)));
}

#[test]
fn kitty_replacement_and_unplaced_deletion_retire_images_and_prefetched_pixels() {
    let mut terminal = terminal();
    terminal.vt_write(CHUNKED);
    let mut state = Capture::new().unwrap();
    let mut prefetch = Prefetch::default();
    prefetch.update(terminal.kitty_graphics());
    let first = frame(&mut terminal, &mut state, &Frame::default(), &prefetch).unwrap();
    let old = first.kitty.images[0].key;
    terminal.vt_write(b"\x1b_Ga=T,q=2,f=24,s=1,v=1,i=7,p=2,C=1;////\x1b\\");
    prefetch.update(terminal.kitty_graphics());
    let next = frame(&mut terminal, &mut state, &first, &prefetch).unwrap();
    assert!(next.kitty.removed.contains(&old));
    assert!(next.kitty.images.iter().all(|image| image.key != old));
    terminal.vt_write(b"\x1b_Ga=d,q=2,d=a\x1b\\");
    let unplaced = frame(&mut terminal, &mut state, &next, &prefetch).unwrap();
    assert!(unplaced.kitty.images.is_empty());
    terminal.vt_write(b"\x1b_Ga=d,q=2,d=I,i=7\x1b\\");
    prefetch.update(terminal.kitty_graphics());
    let gone = frame(&mut terminal, &mut state, &unplaced, &prefetch).unwrap();
    assert!(gone.kitty.generation > unplaced.kitty.generation);
    assert_eq!(gone.kitty.placements_generation, unplaced.kitty.placements_generation);
    assert!(gone.kitty.images.is_empty() && prefetch.images.is_empty());
}

#[test]
fn kitty_relative_chains_use_signed_viewport_positions_and_omit_broken_roots() {
    let mut terminal = terminal();
    for id in 1..=4 {
        image(&mut terminal, id, "");
    }
    terminal.vt_write(b"\x1b[2;3H\x1b_Ga=p,i=1,p=1,c=2,r=2,C=1,q=2\x1b\\");
    terminal.vt_write(b"\x1b_Ga=p,i=2,p=1,P=1,Q=1,H=3,V=2,c=2,r=2,C=1,q=2\x1b\\");
    terminal.vt_write(b"\x1b_Ga=p,i=3,p=1,P=99,Q=1,H=1,V=1,c=1,r=1,C=1,q=2\x1b\\");
    // Omitted Q selects the root's existing placement.
    terminal.vt_write(b"\x1b_Ga=p,i=4,P=1,H=1,V=1,c=1,r=1,C=1,q=2\x1b\\");
    let mut state = Capture::new().unwrap();
    let first = frame(&mut terminal, &mut state, &Frame::default(), &Prefetch::default()).unwrap();
    assert_eq!(at(&first.kitty, 2), [(5, 3)]);
    assert!(at(&first.kitty, 3).is_empty());
    assert_eq!(at(&first.kitty, 4), [(3, 2)]);
    terminal.vt_write(
        b"\x1b[H\x1b_Ga=p,i=1,p=1,c=2,r=2,C=1,q=2\x1b\\\x1b_Ga=p,i=2,p=1,P=1,Q=1,H=-1,V=-1,c=2,r=2,C=1,q=2\x1b\\",
    );
    let next = frame(&mut terminal, &mut state, &first, &Prefetch::default()).unwrap();
    assert_eq!(at(&next.kitty, 2), [(-1, -1)]);
    terminal.vt_write(b"\x1b[2;2H\x1b_Ga=p,i=1,p=1,c=2,r=2,C=1,q=2\x1b\\\x1b_Ga=p,i=2,p=1,P=1,Q=1,H=1,V=1,c=2,r=2,C=1,q=2\x1b\\\x1b_Ga=p,i=3,P=2,Q=1,H=2,V=1,c=1,r=1,C=1,q=2\x1b\\");
    let chained = frame(&mut terminal, &mut state, &next, &Prefetch::default()).unwrap();
    assert_eq!(at(&chained.kitty, 3), [(4, 3)]);
}

#[test]
fn kitty_virtual_roots_use_independent_minima_and_require_visible_cells() {
    let mut terminal = terminal();
    image(&mut terminal, 1, "");
    image(&mut terminal, 2, "");
    terminal.vt_write(b"\x1b_Ga=p,U=1,i=1,p=1,c=2,r=2,q=2\x1b\\\x1b_Ga=p,i=2,P=1,Q=1,H=1,V=2,c=1,r=1,C=1,q=2\x1b\\");
    let mut state = Capture::new().unwrap();
    let empty = frame(&mut terminal, &mut state, &Frame::default(), &Prefetch::default()).unwrap();
    assert!(empty.kitty.placements.is_empty(), "no root cells");
    terminal.vt_write(
        "\x1b[2;4H\x1b[38;5;1;58;5;1m\u{10EEEE}\u{0305}\u{0305}\x1b[3;2H\u{10EEEE}\u{030D}\u{0305}".as_bytes(),
    );
    let shown = frame(&mut terminal, &mut state, &empty, &Prefetch::default()).unwrap();
    assert_eq!(at(&shown.kitty, 2), [(2, 3)]);
    assert_eq!(
        shown.kitty.placements.len(),
        3,
        "two root rows and one child, no duplicate raw placements"
    );
    terminal.vt_write(b"\x1b_Ga=d,d=i,i=1,p=1,q=2\x1b\\");
    let broken = frame(&mut terminal, &mut state, &shown, &Prefetch::default()).unwrap();
    assert!(at(&broken.kitty, 2).is_empty());
}

#[test]
fn kitty_namespaces_collide_without_duplicate_draws_and_prefer_external_virtual_targets() {
    let mut terminal = terminal();
    image(&mut terminal, 1, "");
    image(&mut terminal, 2, "");
    terminal.vt_write(b"\x1b_Ga=p,i=1,C=1,q=2\x1b\\\x1b_Ga=p,i=1,C=1,q=2\x1b\\\x1b_Ga=p,i=1,p=1,U=1,c=2,r=1,q=2\x1b\\");
    terminal.vt_write("\x1b[2;4H\x1b[38;5;1;58;5;1m\u{10EEEE}\u{0305}\u{0305}\u{10EEEE}".as_bytes());
    terminal.vt_write(b"\x1b_Ga=p,i=2,P=1,Q=1,H=1,c=1,r=1,C=1,q=2\x1b\\");
    let mut state = Capture::new().unwrap();
    let first = frame(&mut terminal, &mut state, &Frame::default(), &Prefetch::default()).unwrap();
    let drawn = &first.kitty.placements;
    assert_eq!(
        drawn.len(),
        4,
        "two internal fixed placements, one external placeholder run and its relative child"
    );
    assert_eq!(
        at(&first.kitty, 2),
        [(4, 1)],
        "Q=1 picks the external virtual root over internal placement 1"
    );
    let root = drawn.iter().find(|p| p.image.id == 1 && p.rect.row == 1).unwrap();
    assert_eq!((root.placement_id, root.rect.columns), (1, 2));
    // Unnamed cells prefer the external virtual target over an internal placement with a lower id.
    terminal.vt_write(b"\x1b_Ga=p,i=1,U=1,c=1,r=1,q=2\x1b\\");
    terminal.vt_write("\x1b[3;1H\x1b[59m\u{10EEEE}\u{0305}\u{0305}\u{10EEEE}".as_bytes());
    let next = frame(&mut terminal, &mut state, &first, &Prefetch::default()).unwrap();
    let preferred = next.kitty.placements.iter().find(|p| p.rect.row == 2).unwrap();
    assert_eq!(
        (preferred.placement_id, preferred.rect.columns),
        (0, 2),
        "unnamed cells report id 0 but use the external 2-column grid"
    );
}

#[test]
fn kitty_long_or_garbage_pin_chains_draw_nothing() {
    let mut terminal = terminal();
    for id in 1..=11 {
        image(&mut terminal, id, "");
    }
    terminal.vt_write(b"\x1b_Ga=p,i=1,p=1,c=1,r=1,C=1,q=2\x1b\\");
    for id in 2..=11 {
        terminal.vt_write(format!("\x1b_Ga=p,i={id},p=1,P={},Q=1,c=1,r=1,C=1,q=2\x1b\\", id - 1).as_bytes());
    }
    let mut state = Capture::new().unwrap();
    let first = frame(&mut terminal, &mut state, &Frame::default(), &Prefetch::default()).unwrap();
    assert!(!at(&first.kitty, 2).is_empty());
    assert!(at(&first.kitty, 11).is_empty(), "chain beyond Ghostty's limit");
    terminal.vt_write(b"\x1b[5;1H\n\n\n\n\n\x1b[3J");
    let pruned = frame(&mut terminal, &mut state, &first, &Prefetch::default()).unwrap();
    assert!(
        pruned.kitty.placements.is_empty(),
        "erased-history pins cannot root a chain"
    );
}

#[test]
fn kitty_incomplete_capture_publishes_no_partial_images_or_rows() {
    let mut terminal = terminal();
    terminal.vt_write(CHUNKED);
    let mut state = Capture::new().unwrap();
    let prefetch = Prefetch::default();
    let previous = frame(&mut terminal, &mut state, &Frame::default(), &prefetch).unwrap();
    terminal.vt_write(b"changed\x1b_Ga=d,d=I,i=7,q=2\x1b\\");
    let (pending, kitty) = state.begin_update(&mut terminal, &prefetch).unwrap();
    assert!(kitty.unwrap().placements.is_empty());
    // Even a completed native end cannot publish until its matching image capture is committed.
    pending.end().unwrap();
    assert_eq!(state.convert(&previous, MouseShape::Default), Err(vt::Error::NoValue));
    assert_eq!(previous.kitty.images[0].key.id, 7);
    let recovered = frame(&mut terminal, &mut state, &previous, &prefetch).unwrap();
    assert!(recovered.kitty.images.is_empty() && recovered.kitty.placements.is_empty());
    assert!(recovered.lines()[0].contains("changed"));
}

#[test]
fn kitty_pending_chunks_wait_for_pixels_and_missing_storage_removes_the_view() {
    let mut terminal = terminal();
    let mut state = Capture::new().unwrap();
    let mut prefetch = Prefetch::default();
    let (first, rest) = CHUNKED.split_at(CHUNKED.len() - b"\x1b_Gm=0;Bwg=\x1b\\".len());
    terminal.vt_write(first);
    let loading = frame(&mut terminal, &mut state, &Frame::default(), &prefetch).unwrap();
    assert!(loading.kitty.is_empty());
    terminal.vt_write(rest);
    prefetch.update(terminal.kitty_graphics());
    let shown = frame(&mut terminal, &mut state, &loading, &prefetch).unwrap();
    assert_eq!(shown.kitty.images.len(), 1);
    let mut render = vt::RenderState::new().unwrap();
    render.update(&mut terminal).unwrap();
    let gone = capture(None, 0, render.images().unwrap(), &shown.kitty, &prefetch)
        .unwrap()
        .unwrap();
    assert!(gone.is_empty());
    assert_eq!(gone.removed, [shown.kitty.images[0].key]);
    assert!(
        capture(None, 0, render.images().unwrap(), &gone, &prefetch)
            .unwrap()
            .is_none()
    );
    prefetch.update(None);
    assert!(prefetch.images.is_empty());
}

#[test]
fn kitty_multiple_placements_share_pixels_and_keep_all_three_api_layers() {
    let mut terminal = terminal();
    image(&mut terminal, 1, "");
    for (id, z) in [(1, 0), (2, -1), (3, -1_073_741_825)] {
        terminal.vt_write(format!("\x1b_Ga=p,q=2,i=1,p={id},z={z},C=1\x1b\\").as_bytes());
    }
    let mut state = Capture::new().unwrap();
    let mut prefetch = Prefetch::default();
    prefetch.update(terminal.kitty_graphics());
    let shown = frame(&mut terminal, &mut state, &Frame::default(), &prefetch).unwrap();
    assert_eq!(shown.kitty.images.len(), 1);
    assert_eq!(prefetch.images.len(), 1);
    let mut layers: Vec<_> = shown
        .kitty
        .placements
        .iter()
        .map(|p| (p.placement_id, p.layer))
        .collect();
    layers.sort_by_key(|(id, _)| *id);
    assert_eq!(
        layers,
        [
            (1, KittyLayer::AboveText),
            (2, KittyLayer::BelowText),
            (3, KittyLayer::BelowBackground)
        ]
    );
}

#[test]
fn snapshot_virtual_images_publish_pixels_only_after_their_payload_arrives() {
    let mut source = terminal();
    source.vt_write(b"\x1b_Ga=T,U=1,f=32,s=2,v=2,i=3,c=2,r=1,q=2;AAD//wAA//8AAP//AAD//w==\x1b\\");
    source.vt_write("\x1b[38;5;3m\u{10EEEE}\u{0305}\u{0305}\u{10EEEE}\u{0305}\u{030D}".as_bytes());
    let mut restored =
        vt::Terminal::restore_snapshot(source.encode_snapshot().unwrap(), vt::SnapshotOptions::default()).unwrap();
    let mut state = Capture::new().unwrap();
    let mut prefetch = Prefetch::default();
    prefetch.update(restored.kitty_graphics());
    let pending = frame(&mut restored, &mut state, &Frame::default(), &prefetch).unwrap();
    assert!(pending.kitty.is_empty());
    let Some(vt::SnapshotStep::Image(event)) = restored.restore_snapshot_step().unwrap() else {
        panic!("expected image payload");
    };
    assert_eq!((event.image_id, event.bytes), (3, 16));
    prefetch.update(restored.kitty_graphics());
    let shown = frame(&mut restored, &mut state, &pending, &prefetch).unwrap();
    assert!(shown.kitty.generation > pending.kitty.generation);
    assert_eq!(shown.kitty.images[0].pixels.as_ref(), [0, 0, 255, 255].repeat(4));
    assert_eq!(shown.kitty.placements.len(), 1);
    assert_eq!(shown.kitty.placements[0].rect.columns, 2);
    assert_eq!(restored.restore_snapshot_step(), Ok(None));
}
