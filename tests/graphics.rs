use cryptty::graphics::{DIACRITICS, MARKER, Placeholder, Raster, sender};
use cryptty::terminal::{Content, Terminal};

fn upload(t: &mut Terminal, control: &str, pixels: &[u8]) -> Vec<u8> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    t.feed(
        format!("\x1b_G{control};{}\x1b\\", STANDARD.encode(pixels)).as_bytes(),
    )
}

#[test]
fn split_combining_marks_preserve_following_backspace() {
    let bytes = "a\u{30e}\x08é".as_bytes();
    for split in 0..=bytes.len() {
        let mut t = Terminal::new(10, 2);
        t.feed(&bytes[..split]);
        t.feed(&bytes[split..]);
        assert_eq!(t.cursor, (1, 0), "split at {split}");
        let Content::Text { text } = &t.rows[0][0].content else {
            panic!("expected text");
        };
        assert_eq!(text.chars().collect::<String>(), "é");
    }
}

#[test]
fn sender_survives_every_stream_boundary_and_keeps_image_cells() {
    let raster = Raster {
        width: 2,
        height: 1,
        rgba: vec![255, 0, 0, 255, 0, 255, 0, 128],
    };
    let mut bytes = Vec::new();
    sender::emit(&mut bytes, &raster, 42, 2, 1, false).unwrap();
    for split in 0..=bytes.len() {
        let mut t = Terminal::new(10, 3);
        assert!(t.feed(&bytes[..split]).is_empty());
        assert!(t.feed(&bytes[split..]).is_empty());
        assert_eq!(t.graphics.images[&42].rgba, raster.rgba);
        let p = Placeholder::read(&t.rows[0][1], None).unwrap();
        assert_eq!((p.image, p.row, p.col), (42, 0, 1));
        assert_eq!(t.cursor, (0, 1));
    }
}

#[test]
fn chunked_upload_queries_errors_replacement_and_deletion() {
    let mut t = Terminal::new(20, 3);
    assert_eq!(
        upload(&mut t, "a=q,i=91,f=24,s=1,v=1", &[0, 0, 0]),
        b"\x1b_Gi=91;OK\x1b\\"
    );
    assert!(t.graphics.images.is_empty());
    assert!(
        upload(&mut t, "a=q,i=92,t=s,f=24,s=1,v=1", &[0; 3])
            .starts_with(b"\x1b_Gi=92;ENOTSUP")
    );
    assert!(
        t.feed(b"\x1b_Ga=T,U=1,i=42,f=32,s=2,v=1,m=1;AAAA\x1b\\")
            .is_empty()
    );
    assert_eq!(t.feed(b"\x1b_Gm=0;//////8=\x1b\\"), b"\x1b_Gi=42;OK\x1b\\");
    assert_eq!(t.graphics.images[&42].rgba.len(), 8);
    upload(&mut t, "a=T,U=1,i=42,f=24,s=1,v=1,q=2", &[8, 9, 10]);
    assert_eq!(t.graphics.images[&42].rgba, [8, 9, 10, 255]);
    t.feed(b"\x1b_Ga=d,d=A,q=2\x1b\\");
    assert_eq!(t.graphics.placements.len(), 1);
    t.feed(b"\x1b_Ga=d,d=I,i=42,q=2\x1b\\");
    assert!(t.graphics.images.is_empty());
    assert!(t.graphics.placements.is_empty());
}

#[test]
fn invalid_images_leave_existing_image_unchanged() {
    let mut t = Terminal::new(10, 2);
    upload(&mut t, "a=T,U=1,i=1,f=24,s=1,v=1,q=2", &[1, 2, 3]);
    for control in [
        "a=T,U=1,i=1,f=24,s=4294967295,v=4294967295",
        "a=T,U=1,i=1,f=32,s=1,v=1",
        "a=T,U=1,i=1,f=100",
        "a=T,U=1,i=1,f=24,s=1,v=1,o=z",
        "a=T,U=1,i=1,f=24,s=1,v=1,c=4294967295",
    ] {
        let reply = upload(&mut t, control, &[1, 2, 3]);
        assert!(reply.starts_with(b"\x1b_Gi=1;E"));
        assert_eq!(t.graphics.images[&1].rgba, [1, 2, 3, 255]);
    }
}

#[test]
fn graphics_do_not_leak_into_text_and_cancelled_apcs_are_ignored() {
    let mut t = Terminal::new(30, 3);
    for byte in b"before\x1b_Ga=T,U=1,i=7,f=24,s=1,v=1;AAAA\x18after" {
        t.feed(&[*byte]);
    }
    assert!(t.graphics.images.is_empty());
    let text: String = t.rows[0]
        .iter()
        .filter_map(|cell| match &cell.content {
            Content::Text { text } => Some(text.chars().collect::<String>()),
            _ => None,
        })
        .collect();
    assert!(text.starts_with("beforeafter"), "{text:?}");
    t.feed(b"\x1b]2;title_Gi=7;AAAA\x07");
    assert_eq!(t.title, "title_Gi=7;AAAA");
    assert!(t.graphics.images.is_empty());
}

#[test]
fn placeholders_follow_tmux_style_inheritance_scrolling_and_erasure() {
    let mut t = Terminal::new(8, 2);
    t.feed(
        format!("\x1b[38;2;0;0;42m{MARKER}{}{MARKER}{MARKER}", DIACRITICS[1])
            .as_bytes(),
    );
    let p = Placeholder::read(&t.rows[0][0], None).unwrap();
    assert_eq!((p.row, p.col), (1, 0));
    t.feed(
        format!(
            "\r\n{MARKER}{}{}{MARKER}{MARKER}",
            DIACRITICS[0], DIACRITICS[0]
        )
        .as_bytes(),
    );
    let first = Placeholder::read(&t.rows[1][0], None).unwrap();
    let second = Placeholder::read(&t.rows[1][1], Some(first)).unwrap();
    assert_eq!(second.col, 1);
    t.feed(b"\r\n");
    assert!(Placeholder::read(&t.rows[0][0], None).is_some());
    t.feed(b"\x1b[H\x1b[2J");
    assert!(Placeholder::read(&t.rows[0][0], None).is_none());
}

#[test]
fn png_and_zlib_transmissions_decode_without_filesystem_access() {
    use std::io::Write;
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(512, 1, image::Rgba([70, 90, 130, 255]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let mut t = Terminal::new(10, 2);
    upload(&mut t, "a=T,U=1,i=8,f=100,q=2", png.get_ref());
    assert_eq!(t.graphics.images[&8].width, 512);
    let mut encoder =
        flate2::write::ZlibEncoder::new(Vec::new(), Default::default());
    encoder.write_all(&[255, 128, 64]).unwrap();
    upload(
        &mut t,
        "a=T,U=1,i=9,f=24,s=1,v=1,o=z,q=2",
        &encoder.finish().unwrap(),
    );
    assert_eq!(t.graphics.images[&9].rgba, [255, 128, 64, 255]);
}

#[test]
fn repeated_uploads_evict_old_images_and_allow_new_previews() {
    let mut t = Terminal::new(10, 2);
    for id in 1..=140 {
        upload(
            &mut t,
            &format!("a=T,U=1,i={id},f=24,s=1,v=1,q=2"),
            &[1, 2, 3],
        );
    }
    assert_eq!(t.graphics.images.len(), 128);
    assert!(!t.graphics.images.contains_key(&1));
    assert!(t.graphics.images.contains_key(&140));
    assert_eq!(t.graphics.placements.len(), 128);
}

#[test]
fn replacement_clears_old_placements_after_successful_validation() {
    let mut t = Terminal::new(10, 2);
    upload(&mut t, "a=T,U=1,i=42,p=7,f=24,s=1,v=1,q=2", &[1, 2, 3]);
    upload(&mut t, "a=t,i=42,f=24,s=1,v=1,q=2", &[4, 5, 6]);
    assert!(t.graphics.placements.is_empty());
    upload(&mut t, "a=T,U=1,i=42,p=8,f=24,s=1,v=1,q=2", &[1, 2, 3]);
    upload(&mut t, "a=T,U=1,i=42,p=9,f=24,s=1,v=1,q=2", &[]);
    assert!(t.graphics.placements.contains_key(&(42, 8)));
    assert!(!t.graphics.placements.contains_key(&(42, 9)));
}
