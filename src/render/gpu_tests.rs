use super::{Renderer, painter::Painter};
use crate::terminal::{Position, Terminal, TextRange};

#[test]
#[ignore = "requires a working Metal or Vulkan adapter"]
fn gpu_pixels_match_software_across_resize_and_font_changes() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL | wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(
        instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )
    .expect("GPU adapter required for this explicit test");
    let (device, queue) = pollster::block_on(
        adapter.request_device(&wgpu::DeviceDescriptor::default()),
    )
    .unwrap();
    let mut painter = Painter::new(&device, wgpu::TextureFormat::Rgba8Unorm);
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
        )
        .unwrap();
        painter.clear_glyphs();
        for (width, height, preedit, shape) in [
            (1024, 1024, "", 2),
            (640, 640, "", 4),
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
            terminal.text_blink_phase = shape % 4;
            terminal.selection = Some(TextRange::between(
                Position { row: 0, col: 0 },
                Position { row: 1, col: 5 },
            ));
            terminal.hovered_link = Some(TextRange::between(
                Position { row: 2, col: 0 },
                Position { row: 2, col: 10 },
            ));
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
