//! Ports Ghostty's c/snapshot.zig and snapshot/kitty.zig fixtures at `LIBGHOSTTY_VT_REV` (MIT).
use super::*;
use crate::{FormatOptions, Region};

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/snapshot")
        .join(format!("{name}-v1.hex"));
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .flat_map(|line| line.split('#').next().unwrap().split_whitespace())
        .map(|hex| u8::from_str_radix(hex, 16).unwrap())
        .collect()
}

fn terminal() -> Terminal {
    let mut terminal = Terminal::new(20, 5).unwrap();
    terminal.resize(20, 5, 10, 20).unwrap();
    terminal.set_kitty_image_storage_limit(1_000_000).unwrap();
    terminal
}

fn command(terminal: &mut Terminal, params: &str, pixels: &[u8]) {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(pixels);
    terminal.vt_write(format!("\x1b_G{params};{encoded}\x1b\\").as_bytes());
}

fn pixels(terminal: &mut Terminal, id: u32) -> Vec<u8> {
    terminal
        .kitty_graphics()
        .unwrap()
        .image(id)
        .unwrap()
        .pixels
        .unwrap()
        .to_vec()
}

fn placements(terminal: &mut Terminal) -> Vec<crate::KittyPlacement> {
    let mut values = terminal.kitty_graphics().unwrap().placements().unwrap();
    values.sort_by_key(|p| (p.image_id, p.placement_id));
    values
}

fn image_step(terminal: &mut Terminal) -> SnapshotImage {
    let Some(SnapshotStep::Image(image)) = terminal.restore_snapshot_step().unwrap() else {
        panic!("expected image progress");
    };
    image
}

#[test]
fn ready_reports_images_and_each_payload_has_typed_progress_until_finish() {
    let mut source = terminal();
    source.vt_write(b"\x1b_Ga=T,f=24,i=1,p=1,s=1,v=2,c=4,r=2,q=2;////////\x1b\\\x1b_Ga=T,f=32,i=2,p=3,s=2,v=1,x=1,w=1,X=3,Y=4,z=-1,q=2;AAAA/wAAAP8=\x1b\\");
    let mut restored =
        Terminal::restore_snapshot(source.encode_snapshot().unwrap(), SnapshotOptions::default()).unwrap();
    assert_eq!(
        restored.snapshot_images(),
        Some(SnapshotImages {
            primary: SnapshotImageStorage {
                images: 2,
                pending_bytes: 14
            },
            alternate: None,
        })
    );
    // Geometry is ready but image payloads are still pending.
    assert_eq!(placements(&mut restored), placements(&mut source));
    assert!(restored.kitty_graphics().unwrap().image(1).unwrap().pixels.is_none());
    let mut events = Vec::new();
    for _ in 0..2 {
        events.push(image_step(&mut restored));
        let decoder = restored.history.as_ref().unwrap().decoder.ptr();
        assert_eq!(
            data!(decoder, GHOSTTY_SNAPSHOT_DECODER_DATA_PROGRESS_ROWS, usize),
            Err(Error::NoValue)
        );
        assert_eq!(
            data!(decoder, GHOSTTY_SNAPSHOT_DECODER_DATA_PROGRESS_REMAINING, u32),
            Err(Error::NoValue)
        );
        assert!(restored.history_pending(), "an image step is not FINISH");
    }
    events.sort_by_key(|p| p.image_id);
    assert_eq!(
        events,
        [
            SnapshotImage {
                alternate: false,
                image_id: 1,
                frame: 1,
                bytes: 6
            },
            SnapshotImage {
                alternate: false,
                image_id: 2,
                frame: 1,
                bytes: 8
            },
        ]
    );
    assert_eq!(pixels(&mut restored, 2), [0, 0, 0, 255, 0, 0, 0, 255]);
    assert_eq!(restored.restore_snapshot_step(), Ok(None));
    assert!(!restored.history_pending());
    assert_eq!(restored.snapshot_images(), None);
}

#[test]
fn goldens_decode_fully_and_incrementally_with_the_same_pixels_and_geometry() {
    for name in [
        "kitty-placed",
        "kitty-source-rect",
        "kitty-virtual",
        "kitty-relative",
        "kitty-history",
        "kitty-animation",
        "kitty-loading",
        "kitty-alternate",
        "kitty-payload-in-history",
        "unknown-records",
        "old-pin-complete",
    ] {
        let bytes = fixture(name);
        let mut full = Terminal::decode_snapshot(&bytes, SnapshotOptions::default()).unwrap();
        if name.starts_with("kitty-") && name != "kitty-payload-in-history" {
            assert_eq!(full.encode_snapshot().unwrap(), bytes, "{name}: canonical codec bytes");
        }
        let mut incremental = Terminal::restore_snapshot(bytes, SnapshotOptions::default()).unwrap();
        while incremental.restore_snapshot_step().unwrap().is_some() {}
        assert_eq!(
            (full.cols(), full.rows(), full.total_rows()),
            (incremental.cols(), incremental.rows(), incremental.total_rows()),
            "{name}"
        );
        assert_eq!(
            full.format(Region::Screen, FormatOptions::default()).unwrap(),
            incremental.format(Region::Screen, FormatOptions::default()).unwrap(),
            "{name}"
        );
        for _ in 0..2 {
            assert_eq!(placements(&mut full), placements(&mut incremental), "{name}");
            for id in 1..=6 {
                let a = full
                    .kitty_graphics()
                    .unwrap()
                    .image(id)
                    .map(|i| i.pixels.unwrap().to_vec());
                let b = incremental
                    .kitty_graphics()
                    .unwrap()
                    .image(id)
                    .map(|i| i.pixels.unwrap().to_vec());
                assert_eq!(a, b, "{name} image {id}");
            }
            full.vt_write(b"\x1b[?1049l");
            incremental.vt_write(b"\x1b[?1049l");
        }
    }
}

#[test]
fn unknown_records_are_skipped_and_canonical_history_is_preserved() {
    let bytes = fixture("unknown-records");
    let expected = fixture("kitty-history");
    let full = Terminal::decode_snapshot(&bytes, SnapshotOptions::default()).unwrap();
    assert_eq!(full.encode_snapshot().unwrap(), expected);
    let mut incremental = Terminal::restore_snapshot(bytes, SnapshotOptions::default()).unwrap();
    while incremental.restore_snapshot_step().unwrap().is_some() {}
    assert_eq!(incremental.encode_snapshot().unwrap(), expected);
}

#[test]
fn image_only_compatibility_helper_drains_pixels_and_alternate_totals_are_optional() {
    let mut restored = Terminal::restore_snapshot(fixture("kitty-placed"), SnapshotOptions::default()).unwrap();
    assert_eq!(restored.restore_history_page(), Ok(None));
    assert_eq!(pixels(&mut restored, 1), [255, 0, 0, 255].repeat(64));
    assert!(!restored.history_pending());
    let mut alternate = Terminal::restore_snapshot(fixture("kitty-alternate"), SnapshotOptions::default()).unwrap();
    let totals = alternate.snapshot_images().unwrap();
    assert_eq!(
        totals.primary,
        SnapshotImageStorage {
            images: 1,
            pending_bytes: 4
        }
    );
    assert_eq!(
        totals.alternate,
        Some(SnapshotImageStorage {
            images: 1,
            pending_bytes: 3
        })
    );
    let events = [image_step(&mut alternate), image_step(&mut alternate)];
    assert_eq!(events.map(|e| (e.alternate, e.bytes)), [(true, 3), (false, 4)]);
}

#[test]
fn animation_payloads_arrive_visible_frames_first_and_play_only_after_the_last() {
    let mut source = Terminal::decode_snapshot(&fixture("kitty-animation"), SnapshotOptions::default()).unwrap();
    command(&mut source, "a=T,f=32,s=8,v=8,i=9,C=1,q=2", &[3, 3, 3, 255].repeat(64));
    let mut restored =
        Terminal::restore_snapshot(source.encode_snapshot().unwrap(), SnapshotOptions::default()).unwrap();
    assert_eq!(restored.snapshot_images().unwrap().primary.pending_bytes, 280);
    assert_eq!(restored.kitty_graphics().unwrap().animation_tick(0), Ok(None));
    for (index, (id, frame, bytes)) in [(5, 2, 8), (9, 1, 256), (5, 1, 8), (5, 3, 8)].into_iter().enumerate() {
        assert_eq!(
            image_step(&mut restored),
            SnapshotImage {
                alternate: false,
                image_id: id,
                frame,
                bytes
            }
        );
        assert_eq!(
            restored
                .kitty_graphics()
                .unwrap()
                .animation_tick(1000)
                .unwrap()
                .is_some(),
            index == 3
        );
    }
    assert_eq!(restored.restore_snapshot_step(), Ok(None));
}

#[test]
fn smaller_storage_limits_evict_old_images_and_their_relative_children() {
    let mut source = terminal();
    for id in 1..=3 {
        command(
            &mut source,
            &format!("a=T,f=32,s=4,v=4,i={id},p=1,C=1,q=2"),
            &[5, 5, 5, 255].repeat(16),
        );
    }
    command(&mut source, "a=p,i=3,p=2,P=1,Q=1,H=1,q=2", &[]);
    assert_eq!(placements(&mut source).len(), 4);
    let bytes = source.encode_snapshot().unwrap();
    for (limit, count) in [(150, 2), (10, 0), (0, 0)] {
        let options = SnapshotOptions {
            kitty_image_storage_limit: Some(limit),
            ..SnapshotOptions::default()
        };
        let mut restored = Terminal::restore_snapshot(bytes.clone(), options).unwrap();
        assert_eq!(
            restored.snapshot_images().unwrap().primary,
            SnapshotImageStorage {
                images: count,
                pending_bytes: u64::from(count) * 64
            }
        );
        let decoder = restored.history.as_ref().unwrap().decoder.ptr();
        assert_eq!(
            data!(decoder, GHOSTTY_SNAPSHOT_DECODER_DATA_KITTY_IMAGE_STORAGE_LIMIT, u64),
            Ok(limit)
        );
        while restored.restore_snapshot_step().unwrap().is_some() {}
        assert!(restored.kitty_graphics().unwrap().image(1).is_none());
        assert_eq!(placements(&mut restored).len(), count as usize);
        if count == 2 {
            assert_eq!(pixels(&mut restored, 2).len() + pixels(&mut restored, 3).len(), 128);
            assert_eq!(
                placements(&mut restored).iter().map(|p| p.image_id).collect::<Vec<_>>(),
                [2, 3]
            );
        }
    }
}

#[test]
fn replaced_images_drop_the_old_payload_and_history_placements() {
    let mut restored = Terminal::restore_snapshot(fixture("kitty-history"), SnapshotOptions::default()).unwrap();
    command(&mut restored, "a=t,f=32,s=1,v=1,i=4,q=2", &[0, 0, 0, 255]);
    let event = image_step(&mut restored);
    assert_eq!((event.image_id, event.bytes), (4, 0));
    while restored.restore_history_page().unwrap().is_some() {}
    assert_eq!(pixels(&mut restored, 4), [0, 0, 0, 255]);
    assert!(placements(&mut restored).is_empty());
}

#[test]
fn history_placements_arrive_with_pages_and_resizing_drops_them() {
    for resize in [false, true] {
        let mut restored = Terminal::restore_snapshot(fixture("kitty-history"), SnapshotOptions::default()).unwrap();
        assert_eq!(placements(&mut restored).len(), 1);
        if resize {
            restored.resize(3, 2, 10, 20).unwrap();
        }
        image_step(&mut restored);
        for count in [2, 4] {
            let page = restored.restore_history_page().unwrap().unwrap();
            assert_eq!(page.rows, if resize { 0 } else { 2 });
            assert_eq!(placements(&mut restored).len(), if resize { 1 } else { count });
        }
        assert_eq!(restored.restore_snapshot_step(), Ok(None));
        if resize {
            assert_eq!(placements(&mut restored)[0].placement_id, 3);
        }
    }
    let mut compressed = Terminal::decode_snapshot(
        &fixture("kitty-history"),
        SnapshotOptions {
            compress_history: true,
            ..SnapshotOptions::default()
        },
    )
    .unwrap();
    assert_eq!(placements(&mut compressed).len(), 4);
}

#[test]
fn an_incomplete_chunked_load_resumes_after_its_six_saved_bytes() {
    let mut restored = Terminal::decode_snapshot(&fixture("kitty-loading"), SnapshotOptions::default()).unwrap();
    command(&mut restored, "m=0", &[7, 255, 7, 7, 7, 255, 7, 7, 7, 255]);
    assert_eq!(pixels(&mut restored, 6), [7, 7, 7, 255].repeat(4));
    assert_eq!(placements(&mut restored).len(), 1);
}

// Record framing from Ghostty snapshot/record.zig: tag u16, payload length u32, CRC32C u32.
fn change_record(bytes: &mut Vec<u8>, tag: u16, edit: impl FnOnce(&mut Vec<u8>)) {
    let mut offset = 10;
    loop {
        let length = u32::from_le_bytes(bytes[offset + 2..offset + 6].try_into().unwrap()) as usize;
        if u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap()) == tag {
            let mut payload = bytes[offset + 10..offset + 10 + length].to_vec();
            edit(&mut payload);
            let mut record = tag.to_le_bytes().to_vec();
            record.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
            let mut crc = !0u32;
            for byte in record.iter().chain(&payload) {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    crc = (crc >> 1) ^ (0x82f6_3b78 & (0u32.wrapping_sub(crc & 1)));
                }
            }
            record.extend_from_slice(&(!crc).to_le_bytes());
            record.extend_from_slice(&payload);
            bytes.splice(offset..offset + 10 + length, record);
            return;
        }
        offset += 10 + length;
    }
}

#[test]
fn short_pixel_payloads_and_storage_for_the_wrong_screen_fail() {
    let bytes = fixture("kitty-placed");
    let mut short = bytes.clone();
    change_record(&mut short, 9, |payload| {
        payload.pop();
    });
    assert_eq!(
        Terminal::decode_snapshot(&short, SnapshotOptions::default()).err(),
        Some(Error::InvalidValue)
    );
    let mut incremental = Terminal::restore_snapshot(short, SnapshotOptions::default()).unwrap();
    assert_eq!(incremental.restore_snapshot_step(), Err(Error::InvalidValue));
    assert!(!incremental.history_pending());
    let mut wrong = bytes;
    change_record(&mut wrong, 8, |payload| {
        payload[0] = 1;
    });
    assert_eq!(
        Terminal::decode_snapshot(&wrong, SnapshotOptions::default()).err(),
        Some(Error::InvalidValue)
    );
}
