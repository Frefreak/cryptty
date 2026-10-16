pub(super) fn font_size(
    size: f32,
    cell_width: usize,
    mut advance: impl FnMut(char) -> Option<f32>,
) -> f32 {
    // Measure fixed references in the regular face, never the requested glyph.
    for (ch, columns) in
        [('水', 2.0), ('😀', 2.0), ('M', 1.0), ('0', 1.0), ('▲', 1.0)]
    {
        if let Some(width) = advance(ch).filter(|v| v.is_finite() && *v > 0.0) {
            let adjusted = size * cell_width as f32 * columns / width;
            if adjusted.is_finite() && adjusted > 0.0 {
                return adjusted;
            }
        }
    }
    size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_single_and_double_column_families() {
        let latin = font_size(38.0, 19, |ch| (ch == 'M').then_some(22.88));
        assert!((latin - 31.555944).abs() < 0.001);
        let cjk = font_size(38.0, 19, |ch| match ch {
            '水' => Some(38.0),
            'M' => Some(22.88),
            _ => None,
        });
        assert_eq!(cjk, 38.0);
    }

    #[test]
    fn ignores_missing_and_invalid_reference_advances() {
        assert_eq!(font_size(38.0, 19, |_| None), 38.0);
        assert_eq!(font_size(38.0, 19, |_| Some(f32::NAN)), 38.0);
        assert_eq!(
            font_size(38.0, 19, |ch| match ch {
                '水' => Some(0.0),
                'M' => Some(-1.0),
                '0' => Some(19.0),
                _ => None,
            }),
            38.0
        );
    }
}
