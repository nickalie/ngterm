//! Kitty graphics in the terminal element (plan > Task 16): placement geometry (cells, offsets, scrolling, a viewer's
//! own cells, source rectangles), virtual placements through Unicode placeholders (Ghostty's render tests, the
//! placement a run names in its namespace, relative placements rooted at a virtual placement), the layer order, pixel
//! conversion within the texture budget, the terminal's texture store and its eviction, and over a real pane in GPUI's
//! test app: an image transmitted and placed is drawn from the window's atlas and leaves it when deleted or when the
//! terminal goes, placeholder cells draw their images and the relative placements chained to them, and each Kitty
//! layer is painted at its stage.

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{AppContext as _, Bounds, Entity, TestAppContext, VisualTestContext, point, px, size};
use ngterm_term::{
    KittyFormat, KittyImage, KittyImageKey, KittyLayer, KittyPlacement, KittyView, Pane, PaneCommand, PaneConfig,
    PlacementRect, SourceRect,
};

use super::kitty::textures;
use super::kitty::{
    Cells, DeviceRect, KittyStore, MAX_TEXTURE_PIXELS, MAX_TEXTURE_SIDE, Texture, image_bounds, placements,
    texture_size,
};
use super::{PaintLayer, Terminal, TerminalConfig, TerminalSpec, TerminalView, ViewRole};

fn key(id: u32, generation: u64) -> KittyImageKey {
    KittyImageKey { id, generation }
}

fn image(key: KittyImageKey, width: u32, height: u32) -> Arc<KittyImage> {
    let len = usize::try_from(width * height * 4).unwrap();
    Arc::new(KittyImage {
        key,
        number: 0,
        width,
        height,
        format: KittyFormat::Rgba,
        pixels: vec![0x80; len].into(),
    })
}

fn placed(image: KittyImageKey, z: i32, column: i32, row: i32) -> KittyPlacement {
    KittyPlacement {
        image,
        placement_id: 0,
        layer: if z < i32::MIN / 2 {
            KittyLayer::BelowBackground
        } else if z < 0 {
            KittyLayer::BelowText
        } else {
            KittyLayer::AboveText
        },
        z,
        x_offset: 0,
        y_offset: 0,
        rect: PlacementRect {
            column,
            row,
            columns: 2,
            rows: 1,
            pixel_width: 20,
            pixel_height: 20,
            source: SourceRect {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
            },
        },
    }
}

fn view(images: Vec<Arc<KittyImage>>, placements: Vec<KittyPlacement>) -> KittyView {
    KittyView {
        generation: 1,
        placements_generation: 1,
        images,
        placements,
        removed: Vec::new(),
    }
}

const CELLS: Cells = Cells {
    engine: (10, 20),
    view: (10, 20),
};

#[test]
fn a_placement_lands_at_its_cell_plus_its_offset_inside_the_cell() {
    let a = key(1, 1);
    let mut placement = placed(a, 0, 3, 2);
    placement.x_offset = 4;
    placement.y_offset = 7;
    let drawn = placements(&view(vec![image(a, 4, 4)], vec![placement]), CELLS);
    assert_eq!(
        drawn[0].dest,
        DeviceRect {
            x: 34.0,
            y: 47.0,
            width: 20.0,
            height: 20.0
        }
    );
    // Resolved virtual centering offsets may be larger than one cell; GPUI preserves them.
    placement.x_offset = 500;
    placement.y_offset = 500;
    let clamped = placements(&view(vec![image(a, 4, 4)], vec![placement]), CELLS);
    assert_eq!((clamped[0].dest.x, clamped[0].dest.y), (530.0, 540.0));
}

#[test]
fn a_placement_scrolled_partly_off_the_top_keeps_its_negative_row() {
    let a = key(1, 1);
    let drawn = placements(&view(vec![image(a, 4, 4)], vec![placed(a, 0, 0, -3)]), CELLS);
    assert_eq!((drawn[0].dest.y, drawn[0].dest.height), (-60.0, 20.0));
}

#[test]
fn a_viewer_scales_the_terminals_pixels_by_its_own_cell() {
    let a = key(1, 1);
    let mut placement = placed(a, 0, 2, 1);
    placement.x_offset = 4;
    let half = Cells {
        engine: (10, 20),
        view: (5, 10),
    };
    let drawn = placements(&view(vec![image(a, 4, 4)], vec![placement]), half);
    assert_eq!(
        drawn[0].dest,
        DeviceRect {
            x: 12.0,
            y: 10.0,
            width: 10.0,
            height: 10.0
        }
    );
}

#[test]
fn imageless_and_empty_placements_are_not_drawn() {
    let (a, gone) = (key(1, 1), key(2, 1));
    let mut empty = placed(a, 0, 0, 0);
    empty.rect.pixel_width = 0;
    let shown = placed(a, 5, 1, 1);
    let all = vec![empty, placed(gone, 0, 0, 0), shown];
    let drawn = placements(&view(vec![image(a, 4, 4)], all), CELLS);
    assert_eq!(drawn.len(), 1);
    assert_eq!(drawn[0].z, 5);
}

#[test]
fn placements_draw_by_z_then_image_id_in_three_layers() {
    let (one, two, three) = (key(1, 1), key(2, 1), key(3, 1));
    let images = vec![image(one, 4, 4), image(two, 4, 4), image(three, 4, 4)];
    let below_background = i32::MIN / 2 - 1;
    let all = vec![
        placed(three, 0, 0, 0),
        placed(two, -1, 0, 0),
        placed(one, 0, 0, 0),
        placed(two, below_background, 0, 0),
        placed(one, -1, 1, 0),
        placed(one, -1, 2, 0),
        placed(three, 7, 0, 0),
    ];
    let drawn = placements(&view(images, all), CELLS);
    let order: Vec<(i32, u32, f64)> = drawn
        .iter()
        .map(|placement| (placement.z, placement.image.id, placement.dest.x))
        .collect();
    assert_eq!(
        order,
        [
            (below_background, 2, 0.0),
            // one image at one z keeps the storage's order
            (-1, 1, 10.0),
            (-1, 1, 20.0),
            (-1, 2, 0.0),
            (0, 1, 0.0),
            (0, 3, 0.0),
            (7, 3, 0.0),
        ]
    );
    let layers: Vec<KittyLayer> = drawn.iter().map(|placement| placement.layer).collect();
    assert_eq!(
        layers,
        [
            KittyLayer::BelowBackground,
            KittyLayer::BelowText,
            KittyLayer::BelowText,
            KittyLayer::BelowText,
            KittyLayer::AboveText,
            KittyLayer::AboveText,
            KittyLayer::AboveText,
        ]
    );
}

#[test]
fn the_whole_image_is_scaled_so_its_source_rect_covers_the_placement() {
    let dest = Bounds::new(point(px(100.), px(50.)), size(px(40.), px(20.)));
    // the right half of a 4x2 image, drawn 40x20: twice the image's width, ten times its height
    let source = SourceRect {
        x: 2,
        y: 0,
        width: 2,
        height: 2,
    };
    let whole = image_bounds(dest, source, (4, 2));
    assert_eq!(whole, Bounds::new(point(px(60.), px(50.)), size(px(80.), px(20.))));
    let full = SourceRect {
        x: 0,
        y: 0,
        width: 4,
        height: 2,
    };
    assert_eq!(image_bounds(dest, full, (4, 2)), dest);
}

fn with_pixels(format: KittyFormat, pixels: &[u8], width: u32, height: u32) -> KittyImage {
    KittyImage {
        format,
        pixels: pixels.into(),
        width,
        height,
        ..(*image(key(1, 1), 1, 1)).clone()
    }
}

#[test]
fn pixels_become_bgra_with_straight_alpha() {
    let bgra = |format, pixels: &[u8], width, height| {
        textures::bgra(&with_pixels(format, pixels, width, height)).map(|(pixels, _)| pixels)
    };
    assert_eq!(bgra(KittyFormat::Rgba, &[1, 2, 3, 4], 1, 1), Some(vec![3, 2, 1, 4]));
    assert_eq!(bgra(KittyFormat::Rgb, &[1, 2, 3], 1, 1), Some(vec![3, 2, 1, 255]));
    assert_eq!(
        bgra(KittyFormat::Gray, &[9, 8], 2, 1),
        Some(vec![9, 9, 9, 255, 8, 8, 8, 255])
    );
    assert_eq!(bgra(KittyFormat::GrayAlpha, &[9, 7], 1, 1), Some(vec![9, 9, 9, 7]));
    assert_eq!(bgra(KittyFormat::Rgba, &[1, 2, 3], 1, 1), None, "short");
    assert_eq!(bgra(KittyFormat::Rgb, &[], 0, 0), None, "empty");
    let rendered = textures::render_image(&image(key(1, 1), 3, 2)).unwrap();
    assert_eq!((rendered.size(0).width.0, rendered.size(0).height.0), (3, 2));
}

#[test]
fn a_texture_stays_within_the_atlas_side_and_the_pixel_budget() {
    // libghostty-vt takes up to 10000 px a side
    assert_eq!(texture_size(10_000, 10), (MAX_TEXTURE_SIDE, 8));
    let (width, height) = texture_size(10_000, 6400);
    assert!(u64::from(width) * u64::from(height) <= MAX_TEXTURE_PIXELS);
    assert_eq!((width, height), (5120, 3276));
    assert_eq!(texture_size(3840, 2160), (3840, 2160), "a 4K image keeps its size");
    // a 10000x10000 RGB image (300 MB stored) becomes one 64 MB buffer, sampled straight from the stored pixels
    assert_eq!(texture_size(10_000, 10_000), (4096, 4096));
    // sampling picks the source pixel under each target pixel's center
    let wide = with_pixels(KittyFormat::Gray, &(0..8).collect::<Vec<u8>>(), 8, 1);
    let sampled = textures::resample(&wide.pixels, 1, (8, 1), (4, 1), |pixel| [pixel[0]; 4]).unwrap();
    assert_eq!(
        sampled.chunks(4).map(|pixel| pixel[0]).collect::<Vec<_>>(),
        [1, 3, 5, 7]
    );
}

#[test]
fn a_large_image_converts_into_a_bounded_texture() {
    // 6000x3000 gray: 18 Mpx, over the budget
    let large = with_pixels(KittyFormat::Gray, &vec![200; 6000 * 3000], 6000, 3000);
    let (pixels, (width, height)) = textures::bgra(&large).unwrap();
    assert!(u64::from(width) * u64::from(height) <= MAX_TEXTURE_PIXELS);
    assert_eq!(pixels.len(), usize::try_from(width * height * 4).unwrap());
    assert_eq!(&pixels[..4], &[200, 200, 200, 255]);
}

#[test]
fn the_store_converts_an_image_once_and_forgets_it_with_the_view() {
    let (a, b) = (key(1, 1), key(2, 1));
    let mut store = KittyStore::default();
    let both = view(
        vec![image(a, 4, 4), image(b, 2, 2)],
        vec![placed(a, 0, 0, 0), placed(b, 0, 0, 1)],
    );
    // new images go to the caller to convert (off the main thread) and draw nothing until they arrive
    let new: Vec<KittyImageKey> = store.follow(1, &both).iter().map(|image| image.key).collect();
    assert_eq!(new, [a, b]);
    assert!(store.is_converting(a) && store.get(a).is_none());
    let texture = |image: &Arc<KittyImage>| Texture {
        image: Arc::new(textures::render_image(image).unwrap()),
        size: (image.width, image.height),
    };
    store.finish(a, Some(texture(&both.images[0])));
    store.finish(b, Some(texture(&both.images[1])));
    let first = Arc::clone(&store.get(a).unwrap().image);
    assert_eq!(store.get(b).unwrap().size, (2, 2));
    assert!(store.follow(2, &both).is_empty(), "converted once");
    assert!(
        store.follow(1, &KittyView::default()).is_empty(),
        "an older frame changes nothing"
    );
    assert!(Arc::ptr_eq(&first, &store.get(a).unwrap().image));
    // `a` deleted
    assert!(
        store
            .follow(3, &view(vec![image(b, 2, 2)], vec![placed(b, 0, 0, 1)]))
            .is_empty()
    );
    assert_eq!(store.keys(), [b]);
    // a conversion that ends after its image went is dropped
    store.finish(a, Some(texture(&both.images[0])));
    assert!(store.get(a).is_none());
    // `b` transmitted again under its id: a new generation, a new texture
    let again = key(2, 2);
    let new = store.follow(4, &view(vec![image(again, 2, 2)], vec![placed(again, 0, 0, 1)]));
    assert_eq!(new.len(), 1);
    assert_eq!(store.keys(), [again]);
    // pixels that do not match the size are never drawn
    store.finish(again, None);
    assert!(store.get(again).is_none() && !store.is_converting(again));
}

// ---------------------------------------------------------------------------------------------------------------
// over a real pane

const DEADLINE: Duration = Duration::from_secs(20);

fn terminal(cx: &mut TestAppContext, script: &str, dir: &Path) -> Entity<Terminal> {
    cx.executor().allow_parking();
    let mut command = PaneCommand::new("/bin/sh").arg("-c").arg(script).cwd(dir);
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("NGTERM_") || name.starts_with("AGTERM_") {
            command = command.env_remove(key);
        }
    }
    let spawn = Box::new(move |config: PaneConfig| Pane::spawn(&command, &config));
    cx.new(|cx| Terminal::new(TerminalSpec::default(), TerminalConfig::default(), spawn, cx))
}

fn settle(cx: &mut VisualTestContext, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for the Kitty image");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn drawn(view: &Entity<TerminalView>, cx: &mut VisualTestContext) -> Vec<Arc<gpui_kit::RenderImage>> {
    view.read_with(cx, |view, _| view.images().drawn())
}

/// A 2x1 RGBA image (id 7), sent in two chunks and placed below the text at the cursor, then deleted on a line read
/// (`q=2`: no replies on the input).
const SCRIPT: &str = concat!(
    r"printf '\033_Ga=T,q=2,f=32,s=2,v=1,i=7,z=-1,m=1;AQIDBAUG\033\\\033_Gm=0;Bwg=\033\\'; ",
    r"read -r _; printf '\033_Ga=d,q=2,d=I,i=7\033\\'; exec sleep 5",
);

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn a_placed_image_is_drawn_from_the_atlas_and_leaves_it_when_deleted(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let terminal = terminal(cx, SCRIPT, dir.path());
    let (view, cx) = cx.add_window_view(|_, cx| TerminalView::new(&terminal, ViewRole::Owner, cx));
    cx.simulate_resize(size(px(320.), px(200.)));
    settle(cx, |cx| !drawn(&view, cx).is_empty());
    let image = drawn(&view, cx).remove(0);
    assert_eq!((image.size(0).width.0, image.size(0).height.0), (2, 1));
    assert!(cx.update(|window, _| window.has_image_atlas_entry(&image)));
    view.read_with(cx, |view, _| {
        let frame = view.painted().expect("a frame");
        assert_eq!(frame.kitty.placements[0].layer, KittyLayer::BelowText);
    });
    let shared = terminal.read_with(cx, |terminal, _| Rc::clone(terminal.shared()));
    let painted = view.read_with(cx, |view, _| view.painted().expect("a frame").kitty.images[0].key);
    assert_eq!(shared.kitty_store().keys(), [painted]);

    shared.pane().unwrap().write(b"\n").unwrap();
    settle(cx, |cx| drawn(&view, cx).is_empty());
    assert!(!cx.update(|window, _| window.has_image_atlas_entry(&image)));
    assert!(shared.kitty_store().keys().is_empty());
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn a_torn_down_terminal_drops_its_images_from_the_atlas(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let terminal = terminal(cx, SCRIPT, dir.path());
    let (view, cx) = cx.add_window_view(|_, cx| TerminalView::new(&terminal, ViewRole::Owner, cx));
    cx.simulate_resize(size(px(320.), px(200.)));
    settle(cx, |cx| !drawn(&view, cx).is_empty());
    let image = drawn(&view, cx).remove(0);
    terminal.read_with(cx, |terminal, _| terminal.shared().teardown());
    settle(cx, |cx| drawn(&view, cx).is_empty());
    assert!(!cx.update(|window, _| window.has_image_atlas_entry(&image)));
}

/// Images 1 and 2 (2x1 RGBA) transmitted with virtual placements of one cell (`U=1,c=1,r=1`), then shown through
/// placeholder cells: image 1 named by a 24-bit foreground (0,0,1), image 2 by a palette index (`38;5;2`), each cell
/// U+10EEEE with the row and column diacritics U+0305 U+0305 (0, 0).
const PLACEHOLDERS: &str = concat!(
    r"printf '\033_Ga=T,U=1,q=2,f=32,s=2,v=1,i=1,c=1,r=1;AQIDBAUGBwg=\033\\'; ",
    r"printf '\033_Ga=T,U=1,q=2,f=32,s=2,v=1,i=2,c=1,r=1;AQIDBAUGBwg=\033\\'; ",
    r"printf 'a\033[38;2;0;0;1m\364\216\273\256\314\205\314\205\033[0m\r\n';",
    r"printf 'bc\033[38;5;2m\364\216\273\256\314\205\314\205\033[0m'; exec sleep 5",
);

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn placeholder_cells_draw_their_virtual_placements(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let terminal = terminal(cx, PLACEHOLDERS, dir.path());
    let (view, cx) = cx.add_window_view(|_, cx| TerminalView::new(&terminal, ViewRole::Owner, cx));
    cx.simulate_scale_factor_change(1.0);
    cx.simulate_resize(size(px(320.), px(200.)));
    settle(cx, |cx| drawn(&view, cx).len() == 2);
    let shared = terminal.read_with(cx, |terminal, _| Rc::clone(terminal.shared()));
    let laid = shared.laid.get().expect("laid out");
    let (cell_width, cell_height) = (laid.grid.cell_width, laid.grid.cell_height);
    view.read_with(cx, |view, _| {
        let frame = view.painted().expect("a frame");
        let cells = Cells {
            engine: (cell_width, cell_height),
            view: (cell_width, cell_height),
        };
        let mut drawn = placements(&frame.kitty, cells);
        drawn.sort_by_key(|placement| placement.image.id);
        let at: Vec<(u32, KittyLayer, i32, f64, f64)> = drawn
            .iter()
            .map(|placement| {
                (
                    placement.image.id,
                    placement.layer,
                    placement.z,
                    placement.dest.x,
                    placement.dest.y,
                )
            })
            .collect();
        // the 2x1 image fitted into one cell, centered down it: the run's cell plus the vertical offset
        let width = f64::from(cell_width);
        let offset = ((f64::from(cell_height) - width / 2.0) / 2.0).round();
        let row = f64::from(cell_height);
        assert_eq!(
            at,
            [
                (1, KittyLayer::BelowText, -1, width, offset),
                (2, KittyLayer::BelowText, -1, 2.0 * width, row + offset),
            ]
        );
        assert!(
            drawn
                .iter()
                .all(|placement| (placement.dest.width - width).abs() < 1e-9)
        );
        let layers = view.painted_layers();
        assert!(layers.contains(&(PaintLayer::KittyBelowText, 2)), "{layers:?}");
    });
}

/// Image 1 (2x2 RGBA, at least one source pixel per placeholder row) placed virtually under the client's id 1 on a 2x2 grid (`U=1,p=1,c=2,r=2`); image 2 relative to
/// it (`P=1,Q=1`) three cells right and one down, under id 2; image 3 relative to image 2 (`P=2,Q=2`) one cell left
/// and one down; both one cell and above the text. The placeholder cells name image 1 by a palette index and
/// placement 1 by the underline color (`58;5;1`): grid row 0 at viewport row 1 from column 3, grid row 1 at row 2
/// from column 2 (the second cell of each run a continuation). The root's origin is the minimum column (2) and the
/// minimum row (1) of its runs, so image 2 lands on cell (5, 2) and image 3 on cell (4, 3).
const RELATIVE: &str = concat!(
    r"printf '\033_Ga=T,U=1,q=2,f=32,s=2,v=2,i=1,p=1,c=2,r=2;AQIDBAUGBwgJCgsMDQ4PEA==\033\\'; ",
    r"printf '\033_Ga=t,q=2,f=32,s=2,v=1,i=2;AQIDBAUGBwg=\033\\'; ",
    r"printf '\033_Ga=p,q=2,i=2,p=2,P=1,Q=1,H=3,V=1,c=1,r=1,z=1\033\\'; ",
    r"printf '\033_Ga=t,q=2,f=32,s=2,v=1,i=3;AQIDBAUGBwg=\033\\'; ",
    r"printf '\033_Ga=p,q=2,i=3,P=2,Q=2,H=-1,V=1,c=1,r=1,z=1\033\\'; ",
    r"printf '\033[2;4H\033[38;5;1;58;5;1m\364\216\273\256\314\205\314\205\364\216\273\256\033[0m'; ",
    r"printf '\033[3;3H\033[38;5;1;58;5;1m\364\216\273\256\314\215\314\205\364\216\273\256\033[0m'; exec sleep 5",
);

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn relative_placements_chained_to_a_virtual_placement_land_at_its_cells(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let terminal = terminal(cx, RELATIVE, dir.path());
    let (view, cx) = cx.add_window_view(|_, cx| TerminalView::new(&terminal, ViewRole::Owner, cx));
    cx.simulate_scale_factor_change(1.0);
    cx.simulate_resize(size(px(320.), px(200.)));
    settle(cx, |cx| drawn(&view, cx).len() == 3);
    let shared = terminal.read_with(cx, |terminal, _| Rc::clone(terminal.shared()));
    let laid = shared.laid.get().expect("laid out");
    let (cell_width, cell_height) = (laid.grid.cell_width, laid.grid.cell_height);
    view.read_with(cx, |view, _| {
        let frame = view.painted().expect("a frame");
        let cells = Cells {
            engine: (cell_width, cell_height),
            view: (cell_width, cell_height),
        };
        let at: Vec<(u32, KittyLayer, f64, f64, f64)> = placements(&frame.kitty, cells)
            .iter()
            .filter(|placement| placement.image.id != 1)
            .map(|placement| {
                (
                    placement.image.id,
                    placement.layer,
                    placement.dest.x,
                    placement.dest.y,
                    placement.dest.width,
                )
            })
            .collect();
        let (width, height) = (f64::from(cell_width), f64::from(cell_height));
        assert_eq!(
            at,
            [
                (2, KittyLayer::AboveText, 5.0 * width, 2.0 * height, width),
                (3, KittyLayer::AboveText, 4.0 * width, 3.0 * height, width),
            ]
        );
        let layers = view.painted_layers();
        assert!(layers.contains(&(PaintLayer::KittyAboveText, 2)), "{layers:?}");
    });
}

/// Image 7 below the text (`z=-1`) and image 8 above it (`z=1`), over some text.
const LAYERS: &str = concat!(
    r"printf 'text\r\n'; ",
    r"printf '\033_Ga=T,q=2,f=32,s=2,v=1,i=7,z=-1,C=1;AQIDBAUGBwg=\033\\'; ",
    r"printf '\033_Ga=T,q=2,f=32,s=2,v=1,i=8,z=1,C=1;AQIDBAUGBwg=\033\\'; exec sleep 5",
);

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn each_kitty_layer_is_painted_at_its_stage(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let terminal = terminal(cx, LAYERS, dir.path());
    let (view, cx) = cx.add_window_view(|_, cx| TerminalView::new(&terminal, ViewRole::Owner, cx));
    cx.simulate_resize(size(px(320.), px(200.)));
    settle(cx, |cx| drawn(&view, cx).len() == 2);
    let layers = view.read_with(cx, |view, _| view.painted_layers().to_vec());
    assert_eq!(
        layers,
        [
            (PaintLayer::Background, 0),
            (PaintLayer::KittyBelowBackground, 0),
            (PaintLayer::CellBackgrounds, 0),
            (PaintLayer::KittyBelowText, 1),
            (PaintLayer::Underlines, 0),
            (PaintLayer::Text, 0),
            (PaintLayer::KittyAboveText, 1),
            (PaintLayer::Cursor, 0),
        ]
    );
}

#[gpui_kit::test]
#[cfg_attr(windows, ignore = "drives its pane with a POSIX sh script")]
fn placement_only_replacement_and_deletion_retire_stale_atlas_textures(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let script = concat!(
        r"printf '\033_Ga=T,q=2,f=32,s=2,v=1,i=7,p=1,C=1;AQIDBAUGBwg=\033\\'; read -r _; ",
        r"printf '\033_Ga=T,q=2,f=24,s=1,v=1,i=7,p=1,C=1;////\033\\'; read -r _; ",
        r"printf '\033_Ga=d,q=2,d=i,i=7,p=1\033\\'; read -r _; ",
        r"printf '\033_Ga=d,q=2,d=I,i=7\033\\'; exec sleep 5",
    );
    let terminal = terminal(cx, script, dir.path());
    let (view, cx) = cx.add_window_view(|_, cx| TerminalView::new(&terminal, ViewRole::Owner, cx));
    cx.simulate_resize(size(px(320.), px(200.)));
    settle(cx, |cx| drawn(&view, cx).len() == 1);
    let first = drawn(&view, cx).remove(0);
    let shared = terminal.read_with(cx, |terminal, _| Rc::clone(terminal.shared()));
    shared.pane().unwrap().write(b"\n").unwrap();
    settle(cx, |cx| {
        drawn(&view, cx).first().is_some_and(|image| image.id != first.id)
    });
    assert!(!cx.update(|window, _| window.has_image_atlas_entry(&first)));
    let second = drawn(&view, cx).remove(0);
    assert_eq!((second.size(0).width.0, second.size(0).height.0), (1, 1));
    assert_eq!(shared.kitty_store().keys().len(), 1);
    shared.pane().unwrap().write(b"\n").unwrap();
    settle(cx, |cx| drawn(&view, cx).is_empty());
    assert!(!cx.update(|window, _| window.has_image_atlas_entry(&second)));
    assert!(shared.kitty_store().keys().is_empty());
    let seq = view.read_with(cx, |view, _| view.painted().unwrap().seq);
    shared.pane().unwrap().write(b"\n").unwrap();
    settle(cx, |cx| view.read_with(cx, |view, _| view.painted().unwrap().seq > seq));
    assert!(shared.kitty_store().keys().is_empty());
}
