use std::ffi::OsString;

use anyhow::{Result, bail};

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
    Run {
        renderer: RendererChoice,
        command: Vec<OsString>,
    },
    ListGpus,
    Help,
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Action> {
    let mut args = args.into_iter();
    let mut renderer = RendererChoice::Auto;
    let mut adapter = None;
    let mut command = Vec::new();
    let mut list = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
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
        if renderer != RendererChoice::Auto || !command.is_empty() {
            bail!("Use --list-gpus without renderer or command arguments");
        }
        return Ok(Action::ListGpus);
    }
    Ok(Action::Run { renderer, command })
}

pub const HELP: &str = "Cryptty — terminal for macOS and Linux

Usage: cryptty [OPTIONS] [-- PROGRAM [ARG...]]

  --renderer auto|software|gpu  Renderer (default: auto)
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
                command: vec!["fish".into(), "--no-config".into()],
            }
        );
        assert_eq!(args(&["--list-gpus"]).unwrap(), Action::ListGpus);
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
}
