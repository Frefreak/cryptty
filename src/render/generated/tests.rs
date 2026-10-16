use super::*;

fn glyph(ch: char, w: usize, h: usize) -> Vec<u8> {
    rasterize(ch, w, h, 7).unwrap().data
}

#[test]
fn entire_range_is_cell_aligned_at_small_odd_and_even_sizes() {
    for w in [1, 2, 3, 7, 14, 19, 32] {
        for h in [1, 2, 3, 11, 24, 37, 48] {
            for code in 0x2500..=0x259f {
                let ch = char::from_u32(code).unwrap();
                let image = rasterize(ch, w, h, 7).unwrap();
                assert_eq!(image.data.len(), w * h);
                assert_eq!(image.placement.left, 0);
                assert_eq!(image.placement.top, 7);
                assert!(image.data.iter().any(|&a| a > 0), "{ch} {w}x{h}");
            }
        }
    }
    assert!(rasterize('A', 10, 20, 7).is_none());
}

#[test]
fn blocks_preserve_area_complements_and_shade_density() {
    for (w, h) in [(1, 1), (7, 13), (14, 32), (19, 37)] {
        assert!(glyph('█', w, h).iter().all(|&a| a == 255));
        for (ch, alpha) in [('░', 64), ('▒', 128), ('▓', 192)] {
            assert!(glyph(ch, w, h).iter().all(|&a| a == alpha));
        }
        for (a, b) in [('▀', '▄'), ('▌', '▐'), ('▚', '▞'), ('▘', '▟')]
        {
            for (x, y) in glyph(a, w, h).iter().zip(glyph(b, w, h)) {
                assert!((i32::from(*x) + i32::from(y) - 255).abs() <= 2);
            }
        }
        for (i, ch) in "▁▂▃▄▅▆▇█".chars().enumerate() {
            let area: u32 = glyph(ch, w, h).iter().map(|&a| u32::from(a)).sum();
            let target = (w * h * (i + 1)) as f32 * 255.0 / 8.0;
            assert!((area as f32 - target).abs() <= w as f32);
        }
    }
}

#[test]
fn straight_lines_and_corners_meet_without_seams() {
    for (w, h) in [(7, 13), (14, 32), (19, 37), (32, 48)] {
        for line in ['─', '━', '═'] {
            let data = glyph(line, w, h);
            for row in data.chunks_exact(w) {
                assert!(row.iter().all(|&a| a == row[0]), "{line} {w}x{h}");
            }
        }
        for line in ['│', '┃', '║'] {
            let data = glyph(line, w, h);
            for row in data.chunks_exact(w) {
                assert_eq!(row, &data[..w]);
            }
        }
        for (corner, horizontal, vertical) in [
            ('┌', '─', '│'),
            ('┏', '━', '┃'),
            ('╔', '═', '║'),
            ('╒', '═', '│'),
            ('╓', '─', '║'),
            ('╭', '─', '│'),
        ] {
            let data = glyph(corner, w, h);
            let horizontal = glyph(horizontal, w, h);
            let vertical = glyph(vertical, w, h);
            for y in 0..h {
                assert_eq!(data[y * w + w - 1], horizontal[y * w], "{corner}");
            }
            assert_eq!(&data[(h - 1) * w..], &vertical[..w], "{corner}");
        }
        let cross = glyph('╬', w, h);
        assert_eq!(cross[(h / 2) * w + w / 2], 0);
    }
}

#[test]
fn double_junction_keeps_its_channel_in_a_short_cell() {
    let data = glyph('╬', 32, 3);
    assert_eq!(data[32 + 16], 0);
    assert_eq!(data[32 + 11], 0);
    assert_eq!(data[0], 255);
    assert_eq!(data[64], 255);
    assert_eq!(data[32], 0);
    assert_eq!(data[11], 255);
}
