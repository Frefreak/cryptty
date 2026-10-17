use base64::{Engine, engine::general_purpose::STANDARD};
use cryptty::terminal::Terminal;

fn send(t: &mut Terminal, header: &str, data: &[u8]) -> Vec<u8> {
    t.feed(format!("\x1b_G{header};{}\x1b\\", STANDARD.encode(data)).as_bytes())
}

fn put(t: &mut Terminal, options: &str) -> Vec<u8> {
    send(t, &format!("a=T,f=24,s=1,v=1,{options}"), &[255, 0, 0])
}

#[test]
fn anonymous_png_prints_at_cursor_and_moves_to_bottom_right() {
    let mut t = Terminal::new(40, 20);
    let bytes = concat!(
        "\x1b_Ga=T,f=100,c=10,r=5;",
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ",
        "AAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
        "\x1b\\\n"
    );
    assert!(t.feed(bytes.as_bytes()).is_empty());
    assert_eq!(t.graphics.images.len(), 1);
    assert_eq!(t.graphics.positioned.len(), 1);
    let p = &t.graphics.positioned[0];
    assert_eq!((p.col, p.row), (0, 0));
    assert_eq!((p.layout.cols, p.layout.rows), (10, 5));
    assert_eq!(t.cursor, (10, 5));
    put(&mut t, "c=2,r=2,C=1");
    assert_eq!(t.graphics.images.len(), 2);
    assert_eq!(t.cursor, (10, 5));
}

#[test]
fn named_placements_replace_but_unnamed_placements_accumulate() {
    let mut t = Terminal::new(40, 20);
    assert_eq!(put(&mut t, "i=7,C=1"), b"\x1b_Gi=7;OK\x1b\\");
    send(&mut t, "a=p,i=7,C=1", &[]);
    assert_eq!(t.graphics.positioned.len(), 2);
    send(&mut t, "a=p,i=7,p=9,C=1", &[]);
    t.feed(b"\x1b[4;6H");
    send(&mut t, "a=p,i=7,p=9,C=1", &[]);
    assert_eq!(t.graphics.positioned.len(), 3);
    let p = t.graphics.positioned.last().unwrap();
    assert_eq!((p.col, p.row), (5, 3));
    put(&mut t, "i=7,C=1");
    assert_eq!(t.graphics.positioned.len(), 1);
}

#[test]
fn crop_offsets_scaling_and_signed_layers_are_preserved() {
    let mut t = Terminal::new(40, 20);
    t.cell_pixels = (10, 20);
    let pixels = [255; 10 * 20 * 3];
    send(
        &mut t,
        "a=T,i=1,f=24,s=10,v=20,x=2,y=1,w=3,h=5,X=3,Y=1,c=10,r=1,z=-1,C=1",
        &pixels,
    );
    let p = &t.graphics.positioned[0];
    let g = p.layout.geometry(&t.graphics.images[&1], t.cell_pixels);
    assert_eq!(g.source, [2.0, 1.0, 3.0, 5.0]);
    assert_eq!(p.layout.offset, (3, 1));
    assert_eq!(p.layout.z, -1);
    assert_eq!(g.cells, (10, 1));
    assert!((g.size.0 - 11.4).abs() < 0.001);
    assert_eq!(g.size.1, 19.0);
}

#[test]
fn scrolling_clips_source_without_rescaling() {
    let mut t = Terminal::new(10, 4);
    t.cell_pixels = (10, 20);
    t.feed(b"\x1b[2;4r\x1b[2;1H");
    send(&mut t, "a=T,i=1,f=24,s=10,v=80,c=1,r=2,C=1", &[255; 2400]);
    t.feed(b"\x1b[4;1H\n");
    let p = &t.graphics.positioned[0];
    assert_eq!(p.row, 0);
    assert_eq!(p.clip, (0.5, 1.0));
    assert_eq!(p.layout.rows, 2);
    t.feed(b"\n");
    assert!(t.graphics.positioned.is_empty());
}

#[test]
fn top_aligned_partial_scroll_region_clips_images() {
    let mut t = Terminal::new(10, 5);
    t.cell_pixels = (10, 20);
    t.feed(b"\x1b[1;3r");
    send(&mut t, "a=T,i=1,f=24,s=10,v=80,c=1,r=2,C=1", &[255; 2400]);
    t.feed(b"\x1b[3;1H\n");
    assert_eq!(t.graphics.positioned[0].clip, (0.5, 1.0));
    t.feed(b"\n");
    assert!(t.graphics.positioned.is_empty());
}

#[test]
fn screen_switching_restores_primary_images_and_clear_removes_placements() {
    let mut t = Terminal::new(20, 10);
    put(&mut t, "i=1,C=1");
    t.feed(b"text overwrites cells\x1b[?1049h");
    assert!(t.graphics.positioned.is_empty());
    put(&mut t, "i=2,C=1");
    t.feed(b"\x1b[?1049l");
    assert_eq!(t.graphics.positioned.len(), 1);
    assert_eq!(t.graphics.positioned[0].image, 1);
    t.feed(b"\x1b[2J");
    assert!(t.graphics.positioned.is_empty());
}

#[test]
fn deletion_can_target_a_cell_a_layer_or_a_named_placement() {
    let mut t = Terminal::new(20, 10);
    put(&mut t, "i=1,p=3,c=2,r=2,z=-2,C=1");
    t.feed(b"\x1b[4;4H");
    put(&mut t, "i=2,c=2,r=2,z=3,C=1");
    send(&mut t, "a=d,d=p,x=1,y=1", &[]);
    assert_eq!(t.graphics.positioned.len(), 1);
    assert!(t.graphics.images.contains_key(&1));
    send(&mut t, "a=d,d=Z,z=3", &[]);
    assert!(t.graphics.positioned.is_empty());
    assert!(!t.graphics.images.contains_key(&2));
    send(&mut t, "a=p,i=1,p=4,C=1", &[]);
    send(&mut t, "a=d,d=I,i=1,p=4", &[]);
    assert!(t.graphics.images.is_empty());
}

#[test]
fn image_anchor_follows_reflowed_text_and_history_on_resize() {
    let mut t = Terminal::new(12, 4);
    t.feed(b"abcdefghijklmn");
    put(&mut t, "i=1,C=1");
    assert_eq!(
        (t.graphics.positioned[0].col, t.graphics.positioned[0].row),
        (2, 1)
    );
    t.resize(8, 4);
    assert_eq!(
        (t.graphics.positioned[0].col, t.graphics.positioned[0].row),
        (6, 1)
    );
    t.resize(12, 4);
    assert_eq!(
        (t.graphics.positioned[0].col, t.graphics.positioned[0].row),
        (2, 1)
    );
    t.feed(b"\x1b[4;1H\n\n");
    assert_eq!(t.graphics.positioned[0].row, -1);
    t.resize(12, 6);
    assert_eq!(t.graphics.positioned[0].row, 1);
}

#[test]
fn chunked_placement_uses_cursor_at_final_chunk_and_errors_are_atomic() {
    let mut t = Terminal::new(20, 10);
    t.feed(b"\x1b_Ga=T,i=1,f=24,s=1,v=1,m=1,C=1;/wAA\x1b\\");
    assert!(t.graphics.positioned.is_empty());
    t.feed(b"\x1b[5;7H\x1b_Gm=0;\x1b\\");
    assert_eq!(
        (t.graphics.positioned[0].col, t.graphics.positioned[0].row),
        (6, 4)
    );
    assert!(put(&mut t, "i=1,X=4294967295").starts_with(b"\x1b_Gi=1;EINVAL"));
    assert_eq!(t.graphics.positioned.len(), 1);
    assert_eq!(t.cursor, (6, 4));
}
