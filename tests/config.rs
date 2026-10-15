use std::path::Path;

use cryptty::config::{Config, user_path};
use cryptty::terminal::{PALETTE, Theme};

#[test]
fn partial_theme_overrides_preserve_other_palette_entries() {
    let config = Config::parse(
        r##"
window:
  padding:
    x: 2
colors:
  primary:
    foreground: "#123456"
    background: "0x234567"
  normal:
    blue: "#6272a4"
  bright:
    red: "#ABCDEF"
"##,
    )
    .unwrap();
    let theme = config.colors.theme();
    assert_eq!(theme.foreground, 0x123456);
    assert_eq!(theme.background, 0x234567);
    assert_eq!(theme.cursor, theme.foreground);
    assert_eq!(theme.palette[4], 0x6272a4);
    assert_eq!(theme.palette[9], 0xabcdef);
    for i in (0..16).filter(|i| *i != 4 && *i != 9) {
        assert_eq!(theme.palette[i], PALETTE[i]);
    }
    assert_eq!(config.window.padding.physical(1.25), (2, 0));
    assert_eq!(config.window.padding.physical(2.0), (4, 0));
    for empty in ["", "# Comment-only file", "{}"] {
        assert_eq!(
            Config::parse(empty).unwrap().colors.theme(),
            Theme::default()
        );
    }
}

#[test]
fn malformed_colors_padding_and_unknown_keys_report_errors() {
    for color in ["123456", "#12345", "#1234567", "#-12345", "#aabbgg"] {
        let text = format!("colors:\n  primary:\n    foreground: '{color}'");
        assert!(Config::parse(&text).is_err(), "{color}");
    }
    for text in [
        "window: {padding: {x: -1}}",
        "window: {padding: {x: 65536}}",
        "window: {padding: {x: 1.5}}",
        "window: {padding: {x: '2'}}",
        "window: {paddding: 2}",
        "colors: {primary: {foregound: '#123456'}}",
        "colors: {normal: {blu: '#123456'}}",
        "colors: {unknown: 1}",
        "font: {size: 20}",
        "font: {hinting: 'false'}",
        "font: {hinting: 1}",
        "window: [1, 2]",
        "window: {padding: {x: 1, x: 2}}",
        "{}\n---\n{}",
    ] {
        assert!(Config::parse(text).is_err(), "{text}");
    }
}

#[test]
fn config_path_uses_only_the_user_directory() {
    assert_eq!(
        user_path(Some("/user/config".into()), Some("/user/home".into()))
            .unwrap(),
        Path::new("/user/config/cryptty/cryptty.yaml"),
    );
    for xdg in [None, Some("".into())] {
        assert_eq!(
            user_path(xdg, Some("/user/home".into())).unwrap(),
            Path::new("/user/home/.config/cryptty/cryptty.yaml"),
        );
    }
    assert!(user_path(Some("relative".into()), Some("/home".into())).is_err());
    assert!(user_path(None, Some("relative".into())).is_err());
    assert!(user_path(None, None).is_err());
}

#[test]
fn font_hinting_accepts_boolean_overrides() {
    assert!(!Config::parse("").unwrap().font.hinting);
    assert!(Config::parse("font: {hinting: true}").unwrap().font.hinting);
    assert!(
        !Config::parse("font: {hinting: false}")
            .unwrap()
            .font
            .hinting
    );
}
