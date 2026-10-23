use std::ffi::OsString;

use anyhow::{Result, bail};

use crate::config::OpenGlColorFormat;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RendererChoice {
    #[default]
    Auto,
    Software,
    Gpu {
        adapter: Option<usize>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Image {
        paths: Vec<OsString>,
        pane: bool,
    },
    Run {
        renderer: RendererChoice,
        opengl_color_format: Option<OpenGlColorFormat>,
        command: Vec<OsString>,
    },
    ListGpus,
    Help,
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Action> {
    let mut args = args.into_iter();
    let mut renderer = RendererChoice::Auto;
    let mut adapter = None;
    let mut opengl_color_format = None;
    let mut command = Vec::new();
    let mut list = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("image")
                if renderer == RendererChoice::Auto
                    && adapter.is_none()
                    && !list
                    && opengl_color_format.is_none() =>
            {
                let mut paths = Vec::new();
                let mut pane = false;
                while let Some(arg) = args.next() {
                    match arg.to_str() {
                        Some("--pane") => pane = true,
                        Some("--") => {
                            paths.extend(args);
                            break;
                        }
                        Some("--help" | "-h") => return Ok(Action::Help),
                        Some(value) if value.starts_with('-') => {
                            bail!("Unknown image option {arg:?}; use --help");
                        }
                        _ => paths.push(arg),
                    }
                }
                if paths.is_empty() {
                    bail!("Usage: cryptty image [--pane] FILE [FILE...]");
                }
                return Ok(Action::Image { paths, pane });
            }
            Some("--") => {
                command.extend(args);
                break;
            }
            Some("--help" | "-h") => return Ok(Action::Help),
            Some("--list-gpus") => list = true,
            Some("--renderer") => {
                renderer = match args.next().as_deref().and_then(|s| s.to_str())
                {
                    Some("auto") => RendererChoice::Auto,
                    Some("software") => RendererChoice::Software,
                    Some("gpu") => RendererChoice::Gpu { adapter: None },
                    _ => bail!("--renderer requires auto, software, or gpu"),
                };
            }
            Some("--opengl-color-format") => {
                opengl_color_format = Some(
                    args.next()
                        .and_then(|s| s.into_string().ok())
                        .and_then(|s| s.parse().ok())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "--opengl-color-format requires rgba8 or rgba16f"
                            )
                        })?,
                );
            }
            Some("--gpu") => {
                adapter = Some(
                    args.next()
                        .and_then(|s| s.into_string().ok())
                        .and_then(|s| s.parse::<usize>().ok())
                        .ok_or_else(|| {
                            anyhow::anyhow!("--gpu requires an adapter index")
                        })?,
                );
            }
            _ => bail!("Unknown argument {arg:?}; use --help"),
        }
    }
    if let Some(index) = adapter {
        if renderer == RendererChoice::Software {
            bail!("--gpu conflicts with --renderer software");
        }
        renderer = RendererChoice::Gpu {
            adapter: Some(index),
        };
    }
    if list {
        if renderer != RendererChoice::Auto
            || !command.is_empty()
            || opengl_color_format.is_some()
        {
            bail!(
                "Use --list-gpus without renderer, format, or command arguments"
            );
        }
        return Ok(Action::ListGpus);
    }
    Ok(Action::Run {
        renderer,
        opengl_color_format,
        command,
    })
}

pub const HELP: &str = "Cryptty — terminal for macOS and Linux

Usage: cryptty [OPTIONS] [-- PROGRAM [ARG...]]
       cryptty image [--pane] FILE [FILE...]

  image --pane                Open a right-side tmux preview; Enter closes it

  --renderer auto|software|gpu  Renderer (default: auto)
  --opengl-color-format rgba8|rgba16f
                              Override window.opengl_color_format
                              (default: rgba16f; OpenGL transparency only)
  --list-gpus                  List adapters and exit
  --gpu INDEX                  Require a specific adapter from --list-gpus
  -h, --help                   Show this help

Auto prefers a hardware/virtual GPU and falls back to software.
Explicit GPU requests fail if unavailable. Software skips GPU initialization.
Runs $SHELL by default. Alt+Enter toggles fullscreen.";

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Result<Action> {
        parse(values.iter().map(OsString::from))
    }

    #[test]
    fn gpu_selection_is_explicit_and_shell_arguments_are_preserved() {
        assert_eq!(
            args(&["--gpu", "2", "--", "fish", "--no-config"]).unwrap(),
            Action::Run {
                renderer: RendererChoice::Gpu { adapter: Some(2) },
                opengl_color_format: None,
                command: vec!["fish".into(), "--no-config".into()],
            }
        );
        assert_eq!(args(&["--list-gpus"]).unwrap(), Action::ListGpus);
    }

    #[test]
    fn opengl_format_override_preserves_commands() {
        for (name, expected) in [
            ("rgba8", OpenGlColorFormat::Rgba8),
            ("rgba16f", OpenGlColorFormat::Rgba16f),
        ] {
            assert_eq!(
                args(&["--opengl-color-format", name, "--", "fish"]).unwrap(),
                Action::Run {
                    renderer: RendererChoice::Auto,
                    opengl_color_format: Some(expected),
                    command: vec!["fish".into()],
                }
            );
        }
        for values in [
            vec!["--opengl-color-format"],
            vec!["--opengl-color-format", "rgba32"],
            vec!["--list-gpus", "--opengl-color-format", "rgba8"],
        ] {
            assert!(args(&values).is_err(), "{values:?}");
        }
    }

    #[test]
    fn invalid_selections_are_rejected() {
        for values in [
            vec!["--renderer", "software", "--gpu", "0"],
            vec!["--gpu", "-1"],
            vec!["--gpu"],
            vec!["--renderer", "metal"],
            vec!["--list-gpus", "--", "fish"],
        ] {
            assert!(args(&values).is_err(), "{values:?}");
        }
    }

    #[test]
    fn image_pane_is_optional_and_paths_can_escape_option_parsing() {
        assert_eq!(
            args(&["image", "picture.png"]).unwrap(),
            Action::Image {
                paths: vec!["picture.png".into()],
                pane: false,
            }
        );
        assert_eq!(
            args(&["image", "--pane", "--", "--pane", "a b.png"]).unwrap(),
            Action::Image {
                paths: vec!["--pane".into(), "a b.png".into()],
                pane: true,
            }
        );
        assert!(args(&["image", "--pane"]).is_err());
        assert!(args(&["image", "--unknown", "a.png"]).is_err());
    }
}
