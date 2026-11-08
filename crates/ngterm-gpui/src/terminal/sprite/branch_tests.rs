use super::super::super::metrics::FaceMetrics;
use super::*;

#[test]
fn non_branch_characters_leave_existing_pixels_alone() {
    let m = CellMetrics::calc(&FaceMetrics {
        cell_width: 8.,
        ascent: 16.,
        ..FaceMetrics::default()
    });
    for cp in [0, 0xf5cf, 0xf60e, u32::MAX] {
        let mut canvas = Canvas::new(8, 16);
        canvas.pixel(2, 3, 123);
        draw(cp, &mut canvas, &m);
        let mask = canvas.into_mask();
        assert_eq!((mask.left, mask.top, mask.width, mask.height), (2, 3, 1, 1));
        assert_eq!(mask.alpha, [123]);
    }
}
