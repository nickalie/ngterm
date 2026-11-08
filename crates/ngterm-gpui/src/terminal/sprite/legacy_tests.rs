use super::super::super::metrics::FaceMetrics;
use super::*;

#[test]
fn unassigned_legacy_characters_draw_no_pixels() {
    let m = CellMetrics::calc(&FaceMetrics {
        cell_width: 8.,
        ascent: 16.,
        ..FaceMetrics::default()
    });
    for cp in [0, 0x1fb93, 0x1fbb0, 0x1fbbc, 0x1fbff] {
        let mut canvas = Canvas::new(8, 16);
        draw(cp, &mut canvas, &m);
        assert!(canvas.into_mask().alpha.is_empty(), "{cp:x}");
    }
}
