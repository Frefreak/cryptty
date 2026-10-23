use super::{Renderer, painter::Painter};
use crate::terminal::{Position, Terminal, TextRange};

#[test]
#[ignore = "requires a working GPU adapter"]
fn gpu_pixels_match_software_across_resize_and_font_changes() {
    let instance = super::gpu::instance(
        wgpu::InstanceDescriptor::new_without_display_handle(),
    );
    let adapter = pollster::block_on(
        instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )
    .expect("GPU adapter required for this explicit test");
    let (device, queue) = pollster::block_on(
        adapter.request_device(&wgpu::DeviceDescriptor::default()),
    )
    .unwrap();
    let mut painter = Painter::new(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        adapter.get_info().backend,
        crate::config::OpenGlColorFormat::default(),
    );
    let mut terminal = Terminal::new(26, 16);
    terminal.feed(
        concat!(
            "中文😀e\u{301}\x1b[1;31mBold\x1b[0m\r\n",
            "\x1b[3;2;9mItalic dim strike\x1b[0m\r\n",
            "\x1b[1;3;53mBold italic overline\x1b[0m\r\n",
            "\x1b[4:3;58:2::255:0:0mCurly\x1b[4:2mDouble\x1b[0m\r\n",
            "\x1b[4:4mDotted\x1b[4:5mDashed\x1b[0m\r\n",
            "\x1b[4;44mUnder\x1b[7mInverse\x1b[0m\r\n",
            "\x1b[8mHidden\x1b[0m ABC_gjpq\r\n",
            "\x1b[5mSlow\x1b[6mRapid\x1b[0m\r\n",
            "①❶➀\u{e0b0}⏎▲▼ ██░▒▓ \u{0526}\u{08ab}\u{0afe}\u{324e}\r\n",
        )
        .as_bytes(),
    );
    for start in (0x2500..=0x259f).step_by(26) {
        let row: String = (start..(start + 26).min(0x25a0))
            .map(|code| char::from_u32(code).unwrap())
            .collect();
        terminal.feed(row.as_bytes());
        terminal.feed(b"\r\n");
    }
    terminal.feed(b"\x1b[12;3H");
    let mut image_bytes = Vec::new();
    crate::graphics::sender::emit(
        &mut image_bytes,
        &crate::graphics::Raster {
            width: 2,
            height: 2,
            rgba: vec![
                255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 255, 255, 255, 0, 0,
            ],
        },
        42,
        3,
        2,
        false,
    )
    .unwrap();
    terminal.feed(&image_bytes);
    terminal.feed(
        concat!(
            "\x1b[1;1H\x1b_Ga=T,i=101,f=32,s=2,v=1,c=6,r=2,C=1,z=-2147483648;",
            "/wAAgAD/AP8=\x1b\\",
            "\x1b[2;2H\x1b_Ga=p,i=101,p=1,c=4,r=3,C=1,z=-1,X=1,Y=1\x1b\\",
            "\x1b[2;2H\x1b_Ga=T,i=102,f=24,s=1,v=1,c=3,r=2,C=1,z=-2;AAD/\x1b\\",
            "\x1b[2;2H\x1b_Ga=p,i=101,p=3,c=4,r=3,C=1,z=-3\x1b\\",
            "\x1b[3;3H\x1b_Ga=p,i=101,p=2,c=3,r=2,C=1,z=1\x1b\\",
            "\x1b[1;2H\x1b[44m  \x1b[0m"
        )
        .as_bytes(),
    );
    terminal.feed(b"\x1b[H");
    let mapped_family = if cfg!(target_os = "macos") {
        "Menlo"
    } else {
        "DejaVu Sans Mono"
    };
    let config = crate::config::Config::parse(&format!(
        "font: {{codepoint_map: [{{ranges: ['U+23CE', 'U+2580-U+25FF'], family: {mapped_family:?}}}]}}"
    )).unwrap();
    let defaults = crate::config::Font::default();
    for (font_size, mapped) in
        [(19.0, false), (23.75, true), (38.0, false), (38.0, true)]
    {
        let mut renderer = Renderer::with_options(
            font_size,
            (3, 5),
            false,
            &defaults.family,
            if mapped {
                &config.font.codepoint_map
            } else {
                &defaults.codepoint_map
            },
            &defaults.fallback,
            Default::default(),
        )
        .unwrap();
        painter.clear_glyphs();
        for (width, height, preedit, shape) in [
            (1024, 1024, "", 2),
            (640, 640, "", 4),
            (640, 640, "", 8),
            (256, 90, "", 8),
            (1, 1, "", 8),
            (384, 160, "测试", 2),
            (256, 90, "", 6),
            (1, 1, "中", 6),
        ] {
            if shape == 4 {
                let mut theme = *terminal.theme();
                theme.foreground = 0xbbaa99;
                theme.background = 0x123456;
                theme.palette[1] = 0x55aa77;
                terminal.set_theme(theme);
            }
            terminal.feed(format!("\x1b[{shape} q").as_bytes());
            terminal.vim_mode = shape == 8;
            terminal.text_blink_phase = shape % 4;
            terminal.selection = Some(TextRange::between(
                Position { row: 0, col: 0 },
                Position { row: 1, col: 5 },
            ));
            terminal.hovered_link = vec![TextRange::between(
                Position { row: 2, col: 0 },
                Position { row: 2, col: 10 },
            )];
            renderer.set_link_tooltip(
                (shape == 6).then_some("file:///tmp/中文/test%20file.txt"),
                (20, 3),
            );
            let targets = if shape == 8 {
                crate::marker::Session::new(
                    terminal.visible_snapshot(),
                    Vec::new(),
                    &crate::marker::Options::default(),
                )
                .visible_targets()
            } else {
                Vec::new()
            };
            renderer.set_markers(targets);
            renderer.draw(&terminal, width, height, preedit);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("pixel comparison"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            painter
                .draw(
                    &device,
                    &queue,
                    &texture.create_view(&Default::default()),
                    &mut renderer,
                    &terminal,
                    (width, height),
                    preedit,
                )
                .unwrap();
            let stride = (width * 4).div_ceil(256) * 256;
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pixel readback"),
                size: (stride * height) as u64,
                usage: wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder =
                device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(stride),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            let (send, receive) = std::sync::mpsc::channel();
            buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                send.send(r).unwrap();
            });
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            receive.recv().unwrap().unwrap();
            let data = buffer.slice(..).get_mapped_range().unwrap();
            for y in 0..height as usize {
                for x in 0..width as usize {
                    let expected = renderer.pixels[y * width as usize + x];
                    for (channel, shift) in [16, 8, 0].into_iter().enumerate() {
                        let actual =
                            data[y * stride as usize + x * 4 + channel];
                        let expected = ((expected >> shift) & 255) as u8;
                        assert!(
                            actual.abs_diff(expected) <= 2,
                            "font={font_size} size={width}x{height} at {x},{y} channel={channel}: GPU={actual} CPU={expected}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
#[ignore = "requires a working GPU adapter"]
fn transparent_pixels_and_opaque_restoration() {
    check_transparent_pixels(crate::config::OpenGlColorFormat::Rgba16f);
}

#[test]
#[ignore = "requires a working GPU adapter"]
fn compact_transparent_pixels_and_opaque_restoration() {
    check_transparent_pixels(crate::config::OpenGlColorFormat::Rgba8);
}

fn check_transparent_pixels(color_format: crate::config::OpenGlColorFormat) {
    let instance = super::gpu::instance(
        wgpu::InstanceDescriptor::new_without_display_handle(),
    );
    let adapter = pollster::block_on(
        instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )
    .unwrap();
    let (device, queue) = pollster::block_on(
        adapter.request_device(&wgpu::DeviceDescriptor::default()),
    )
    .unwrap();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut painter =
        Painter::new(&device, format, adapter.get_info().backend, color_format);
    let defaults = crate::config::Font::default();
    let mut renderer = Renderer::with_options(
        19.0,
        (0, 0),
        false,
        &defaults.family,
        &[],
        &[],
        0.15.try_into().unwrap(),
    )
    .unwrap();
    let mut terminal = Terminal::new(6, 3);
    terminal.feed(b"A\x1b[41m \x1b[0m\x1b[?25l");
    terminal.feed(
        b"\x1b[1;4H\x1b[38;2;122;122;122mA\x1b[38;2;99;168;248mA\x1b[97mA\x1b[0m",
    );
    terminal.feed(b"\x1b[2;1H");
    let mut image = Vec::new();
    crate::graphics::sender::emit(
        &mut image,
        &crate::graphics::Raster {
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 128],
        },
        123,
        1,
        1,
        false,
    )
    .unwrap();
    terminal.feed(&image);

    let (cw, ch) = (renderer.cell_width, renderer.cell_height);
    let origin = renderer.cell_origin(0, 0);
    let mut opaque = Vec::new();
    let mut glyph_coverage = Vec::new();
    for (alpha, size) in [
        (0.0, (256, 128)),
        (1.0, (256, 128)),
        (0.8, (256, 128)),
        (0.4, (256, 128)),
        (0.05, (256, 128)),
        (0.001, (256, 128)),
        (0.82, (512, 192)),
        (0.0, (128, 64)),
        (1.0, (256, 128)),
    ] {
        painter.set_opacity(&device, format, alpha.try_into().unwrap());
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("transparency test"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        painter
            .draw(
                &device,
                &queue,
                &texture.create_view(&Default::default()),
                &mut renderer,
                &terminal,
                size,
                "",
            )
            .unwrap();
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("alpha readback"),
            size: u64::from(size.0 * size.1 * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size.0 * 4),
                    rows_per_image: Some(size.1),
                },
            },
            texture.size(),
        );
        queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            send.send(r).unwrap();
        });
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        receive.recv().unwrap().unwrap();
        let bytes = buffer.slice(..).get_mapped_range().unwrap();
        if glyph_coverage.is_empty() {
            // With no background alpha, these isolated glyphs expose the mask.
            glyph_coverage.extend(bytes.chunks_exact(4).map(|p| p[3]));
        }
        let pixel = |x: usize, y: usize| {
            let offset = (y * size.0 as usize + x) * 4;
            &bytes[offset..offset + 4]
        };
        let background = super::scene::color(terminal.theme().background);
        for (channel, value) in background.into_iter().enumerate() {
            let expected = (value * alpha * 255.0).round() as u8;
            assert!(
                pixel(size.0 as usize - 1, size.1 as usize - 1)[channel]
                    .abs_diff(expected)
                    <= 1
            );
        }
        let image_pixel =
            pixel(origin.0 + cw / 2, origin.1 + ch + cw.min(ch) / 2);
        for channel in 0..4 {
            let source = if channel == 0 || channel == 3 {
                128.0
            } else {
                0.0
            };
            let expected = if alpha < 1.0 && channel < 3 {
                let a = 128.0 / 255.0 + alpha * 127.0 / 255.0;
                let linear = source / 255.0
                    + to_linear(background[channel]) * alpha * 127.0 / 255.0;
                (to_srgb(linear / a) * a * 255.0).round() as u8
            } else {
                (source + background[channel] * alpha * 127.0).round() as u8
            };
            assert!(
                image_pixel[channel].abs_diff(expected) <= 2,
                "image alpha={alpha} channel={channel}: {image_pixel:?}"
            );
        }
        if alpha < 1.0 {
            let mut edges = 0;
            let white = super::scene::color(terminal.theme().palette[15]);
            for (col, foreground) in [
                (3, [122.0 / 255.0; 3]),
                (4, [99.0 / 255.0, 168.0 / 255.0, 248.0 / 255.0]),
                (5, [white[0], white[1], white[2]]),
            ] {
                for y in origin.1..origin.1 + ch {
                    for x in origin.0 + col * cw..origin.0 + (col + 1) * cw {
                        let rgba = pixel(x, y);
                        let a = f32::from(rgba[3]) / 255.0;
                        let coverage =
                            f32::from(glyph_coverage[y * 256 + x]) / 255.0;
                        let expected_alpha =
                            coverage + alpha * (1.0 - coverage);
                        assert!((a - expected_alpha).abs() <= 1.0 / 255.0);
                        if !(0.1..0.9).contains(&coverage) {
                            continue;
                        }
                        edges += 1;
                        for backdrop in [0.0, 0.5, 1.0] {
                            for channel in 0..3 {
                                let actual = to_srgb(
                                    to_linear(
                                        f32::from(rgba[channel]) / 255.0 / a,
                                    ) * a
                                        + to_linear(backdrop) * (1.0 - a),
                                );
                                let background = to_linear(background[channel])
                                    * alpha
                                    + to_linear(backdrop) * (1.0 - alpha);
                                let expected = to_srgb(
                                    to_linear(foreground[channel]) * coverage
                                        + background * (1.0 - coverage),
                                );
                                assert!(
                                    (actual - expected).abs() < 0.016,
                                    "glyph alpha={alpha} backdrop={backdrop} coverage={coverage} channel={channel}: actual={actual} expected={expected}"
                                );
                            }
                        }
                    }
                }
            }
            assert!(
                edges > 0,
                "Must test partially covered colored glyph edges"
            );
        }
        // Explicit ANSI backgrounds stay opaque.
        assert_eq!(pixel(origin.0 + cw, origin.1)[3], 255);
        let mut ink = 0;
        for y in origin.1..origin.1 + ch {
            for x in origin.0..origin.0 + cw {
                let rgba = pixel(x, y);
                assert!(rgba[..3].iter().all(|&c| c <= rgba[3]));
                if alpha < 1.0 && rgba[3] == 255 {
                    ink += 1;
                }
            }
        }
        if alpha < 1.0 {
            assert!(ink > 0, "Text must retain opaque ink");
        } else if opaque.is_empty() {
            opaque = bytes.to_vec();
        } else {
            assert_eq!(&*bytes, opaque.as_slice());
        }
    }
}

fn to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb(value: f32) -> f32 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}
