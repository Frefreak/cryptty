use std::path::Path;

use cryptty::config::{Config, user_path};
use cryptty::terminal::{PALETTE, Theme};

const DEFAULT_PIXELS_AT_1_5X: f32 = if cfg!(target_os = "macos") {
    28.5
} else {
    38.0
};

struct TestDirectory(std::path::PathBuf);

impl TestDirectory {
    fn new() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir()
            .join(format!("cryptty-config-{}-{id}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn missing_environment_warns_and_uses_defaults() {
    const CHILD: &str = "CRYPTTY_CONFIG_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let config = Config::load().unwrap();
        assert_eq!(config.colors.theme(), Theme::default());
        assert!(!config.font.hinting);
        assert_eq!(config.font.size.pixels(1.5), DEFAULT_PIXELS_AT_1_5X);
        assert_eq!(config.window.padding.physical(1.0), (2, 2));
        return;
    }
    for empty in [false, true] {
        let mut command =
            std::process::Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "missing_environment_warns_and_uses_defaults",
            "--nocapture",
        ]);
        command.env(CHILD, "1");
        for key in ["HOME", "XDG_CONFIG_HOME"] {
            if empty {
                command.env(key, "");
            } else {
                command.env_remove(key);
            }
        }
        let output = command.output().unwrap();
        assert!(output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("Warning: HOME and XDG_CONFIG_HOME"));
        assert!(stderr.contains("using built-in defaults"));
    }
}

#[test]
fn missing_config_is_created_with_current_defaults() {
    let directory = TestDirectory::new();
    let path = directory.path().join("nested/cryptty/cryptty.yaml");
    let loaded = Config::read(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.lines().all(|line| line.starts_with('#')));
    let saved = Config::read(&path).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    for config in [loaded, saved] {
        assert_eq!(config.colors.theme(), Theme::default());
        assert!(!config.font.hinting);
        assert_eq!(config.font.size.pixels(1.5), DEFAULT_PIXELS_AT_1_5X);
        assert_eq!(config.window.padding.physical(1.0), (2, 2));
    }
}

#[test]
fn existing_config_is_preserved_even_when_invalid() {
    let directory = TestDirectory::new();
    let path = directory.path().join("cryptty.yaml");
    for (text, valid) in [("font: {hinting: true}\n", true), ("font: [", false)]
    {
        std::fs::write(&path, text).unwrap();
        let result = Config::read(&path);
        assert_eq!(result.is_ok(), valid);
        if let Ok(config) = result {
            assert!(config.font.hinting);
        }
        let updated = std::fs::read_to_string(&path).unwrap();
        if valid {
            assert!(updated.starts_with(text));
            assert!(!updated.contains("#   hinting:"));
            assert!(updated.contains("#   size: 19"));
            Config::read(&path).unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), updated);
        } else {
            assert_eq!(updated, text);
        }
    }
}

#[test]
fn config_directory_errors_are_reported() {
    let directory = TestDirectory::new();
    let parent = directory.path().join("file");
    std::fs::write(&parent, "occupied").unwrap();
    assert!(Config::read(&parent.join("cryptty.yaml")).is_err());
}

#[test]
fn simultaneous_startups_append_defaults_once() {
    for existing in [false, true] {
        let directory = TestDirectory::new();
        let path = directory.path().join("cryptty.yaml");
        if existing {
            std::fs::write(&path, "font: {size: 24}\n").unwrap();
        }
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let path = &path;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    let config = Config::read(path).unwrap();
                    let pixels = if !existing {
                        DEFAULT_PIXELS_AT_1_5X
                    } else if cfg!(target_os = "macos") {
                        36.0
                    } else {
                        48.0
                    };
                    assert_eq!(config.font.size.pixels(1.5), pixels);
                });
            }
        });
        let text = std::fs::read_to_string(path).unwrap();
        assert_eq!(text.matches("#   hinting: false").count(), 1);
    }
}

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
    assert_eq!(config.window.padding.physical(1.25), (2, 2));
    assert_eq!(config.window.padding.physical(2.0), (4, 4));
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
        "font: {sizes: 20}",
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
fn font_size_uses_points_and_scales_with_the_display() {
    let cases = if cfg!(target_os = "macos") {
        [("12", 12.0), ("11.25", 11.25), ("19", 19.0)]
    } else {
        [("12", 16.0), ("11.25", 15.0), ("19.5", 26.0)]
    };
    for (value, pixels) in cases {
        let config =
            Config::parse(&format!("font: {{size: {value}}}")).unwrap();
        assert_eq!(config.font.size.pixels(1.0), pixels);
        assert_eq!(config.font.size.pixels(2.0), pixels * 2.0);
    }
    for value in ["0", "-1", ".nan", ".inf", "-.inf", "'19'", "true"] {
        assert!(Config::parse(&format!("font: {{size: {value}}}")).is_err());
    }
    let config = Config::parse("window: {padding: {x: 0, y: 0}}").unwrap();
    assert_eq!(config.window.padding.physical(2.0), (0, 0));
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

#[test]
fn font_family_is_configurable_and_defaults_to_the_alacritty_family() {
    let default = Config::parse("").unwrap();
    assert_eq!(default.font.family, "Alacritty Fantasque WenKai Mono");
    let config = Config::parse("font: {family: Menlo}").unwrap();
    assert_eq!(config.font.family, "Menlo");
    assert_eq!(config.font.size.pixels(1.5), DEFAULT_PIXELS_AT_1_5X);
    assert!(Config::parse("font: {family: [Menlo]}").is_err());
}

#[test]
fn font_mappings_accept_multiple_disjoint_ranges() {
    let config = Config::parse(
        r#"
font:
  codepoint_map:
    - ranges: ["U+23CE", "U+25B2-U+25BC", "U+10FFFF"]
      family: " DejaVu Sans Mono "
    - ranges: ["U+2580-U+259F"]
      family: Menlo
"#,
    )
    .unwrap();
    let maps = &config.font.codepoint_map;
    assert_eq!(maps.len(), 2);
    assert_eq!(maps[0].family(), "DejaVu Sans Mono");
    for ch in ['⏎', '▲', '▼', '\u{10ffff}'] {
        assert!(maps[0].contains(ch), "{ch:?}");
    }
    for ch in ['A', '█', '░'] {
        assert!(!maps[0].contains(ch), "{ch:?}");
    }
    assert!(maps[1].contains('█'));
    assert!(maps[1].contains('░'));
    assert!(!maps[1].contains('⏎'));
    assert!(Config::default().font.codepoint_map.is_empty());
}

#[test]
fn invalid_font_mapping_ranges_are_rejected_during_parsing() {
    for range in [
        "",
        "23CE",
        "U+",
        "U+-1",
        "U+nope",
        "U+110000",
        "U+D800",
        "U+DFFF",
        "U+25BC-U+25B2",
        "U+0041-U+D800",
        "U++0041",
        "U+0041-U+0042-U+0043",
    ] {
        let yaml = format!(
            "font: {{codepoint_map: [{{ranges: [{range:?}], family: Menlo}}]}}"
        );
        assert!(Config::parse(&yaml).is_err(), "{range}");
    }
    for mapping in [
        "{ranges: [], family: Menlo}",
        "{ranges: ['U+0041'], family: '  '}",
        "{ranges: ['U+0041'], family: [Menlo]}",
        "{ranges: 'U+0041', family: Menlo}",
        "{ranges: ['U+0041'], familiy: Menlo}",
        "{ranges: ['U+0041']}",
    ] {
        let yaml = format!("font: {{codepoint_map: [{mapping}]}}");
        assert!(Config::parse(&yaml).is_err(), "{mapping}");
    }
}
