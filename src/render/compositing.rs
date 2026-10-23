//! Premultiplied sRGB pixels, with source-over blending in linear light.

pub(super) fn premultiply(rgb: u32, alpha: f32) -> u32 {
    let channel = |shift: u32| {
        ((((rgb >> shift) & 255) as f32 * alpha).round() as u32) << shift
    };
    ((alpha * 255.0).round() as u32) << 24
        | channel(16)
        | channel(8)
        | channel(0)
}

pub(super) fn blend(bg: u32, fg: u32, alpha: u8, transparent: bool) -> u32 {
    if !transparent {
        return super::blend(bg, fg, alpha);
    }
    if alpha == 0 {
        return bg;
    }
    if alpha == 255 {
        return fg | 0xff000000;
    }
    let source_alpha = f32::from(alpha) / 255.0;
    let dest_alpha = (bg >> 24) as f32 / 255.0;
    let remaining = dest_alpha * (1.0 - source_alpha);
    let out_alpha = source_alpha + remaining;
    let channel = |shift: u32| {
        let source = ((fg >> shift) & 255) as f32 / 255.0;
        let dest = if dest_alpha > 0.0 {
            (((bg >> shift) & 255) as f32 / 255.0 / dest_alpha).min(1.0)
        } else {
            0.0
        };
        let linear = (decode(source) * source_alpha + decode(dest) * remaining)
            / out_alpha;
        ((encode(linear) * out_alpha * 255.0).round() as u32) << shift
    };
    ((out_alpha * 255.0).round() as u32) << 24
        | channel(16)
        | channel(8)
        | channel(0)
}

fn decode(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn encode(value: f32) -> f32 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_preserves_foreground_and_premultiplied_alpha() {
        assert_eq!(premultiply(0xffffff, 0.0), 0);
        assert_eq!(blend(0, 0xffffff, 128, true), 0x80808080);
        assert_eq!(blend(0x80402010, 0xffffff, 0, true), 0x80402010);
        assert_eq!(blend(0x80402010, 0x123456, 255, true), 0xff123456);
        // Half white over opaque black gives sRGB 188 after linear blending.
        assert_eq!(blend(0xff000000, 0xffffff, 128, true), 0xffbcbcbc);
    }
}
