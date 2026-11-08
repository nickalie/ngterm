use super::super::super::metrics::FaceMetrics;
use super::*;

fn metrics() -> CellMetrics {
    CellMetrics::calc(&FaceMetrics {
        cell_width: 8.,
        ascent: 16.,
        ..FaceMetrics::default()
    })
}

#[test]
fn block_elements_cover_their_documented_fraction_of_the_cell() {
    let m = metrics();
    for (cp, left, top, width, height) in [
        (0x2580, 0, 0, 8, 8),
        (0x2581, 0, 14, 8, 2),
        (0x258c, 0, 0, 4, 16),
        (0x2590, 4, 0, 4, 16),
        (0x2594, 0, 0, 8, 2),
        (0x2595, 7, 0, 1, 16),
        (0x2596, 0, 8, 4, 8),
        (0x2597, 4, 8, 4, 8),
        (0x2598, 0, 0, 4, 8),
        (0x259d, 4, 0, 4, 8),
    ] {
        let mut canvas = Canvas::new(8, 16);
        draw(cp, &mut canvas, &m);
        let mask = canvas.into_mask();
        assert_eq!(
            (mask.left, mask.top, mask.width, mask.height),
            (left, top, width, height),
            "{cp:x}"
        );
        assert!(mask.alpha.iter().all(|alpha| *alpha == 255));
    }
}

#[test]
fn unknown_code_points_leave_the_canvas_empty() {
    for cp in [0, 0x257f, 0x25a0, u32::MAX] {
        let mut canvas = Canvas::new(8, 16);
        draw(cp, &mut canvas, &metrics());
        assert!(canvas.into_mask().alpha.is_empty());
    }
}
