use std::ffi::OsString;
use std::fs::File;
use std::io::{self, IsTerminal, Read, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::ImageEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};

use super::{DIACRITICS, MAX_BYTES, Raster};

pub fn open_pane(paths: &[OsString]) -> Result<()> {
    ensure!(
        !paths.is_empty(),
        "Usage: cryptty image --pane FILE [FILE...]"
    );
    ensure!(
        std::env::var_os("TMUX").is_some_and(|v| !v.is_empty()),
        "Image --pane requires a tmux session"
    );
    let pane = std::env::var_os("TMUX_PANE")
        .filter(|pane| !pane.is_empty())
        .context("TMUX_PANE is required to open an image preview")?;
    let paths = paths
        .iter()
        .map(|path| {
            std::fs::canonicalize(path)
                .with_context(|| format!("Open {}", Path::new(path).display()))
        })
        .collect::<Result<Vec<_>>>()?;
    let layout = Command::new("tmux")
        .args(["display-message", "-p", "-t"])
        .arg(&pane)
        .arg("#{window_layout}")
        .stdin(Stdio::null())
        .output()
        .context("Read tmux layout before image preview")?;
    ensure!(
        layout.status.success(),
        "Read tmux layout: {}",
        String::from_utf8_lossy(&layout.stderr).trim()
    );
    let layout = String::from_utf8(layout.stdout)?;
    let selected = Command::new("tmux")
        .args(["select-window", "-t"])
        .arg(&pane)
        .stdin(Stdio::null())
        .output()
        .context("Show tmux window for image preview")?;
    ensure!(
        selected.status.success(),
        "Show tmux window: {}",
        String::from_utf8_lossy(&selected.stderr).trim()
    );
    let output = Command::new("tmux")
        .args(["split-window", "-h", "-f", "-l", "35%", "-t"])
        .arg(&pane)
        .args(["-P", "-F", "#{pane_id}", "/bin/sh", "-c"])
        .arg(concat!(
            "source_pane=$1; layout=$2; shift 2; ",
            "tmux set -p -t \"$TMUX_PANE\" remain-on-exit off || exit; ",
            "CRYPTTY_IMAGE_PREVIEW=1 \"$@\"; ",
            "printf '\\nPress Enter to close preview.\\n'; ",
            "read -r reply; ",
            "tmux kill-pane -t \"$TMUX_PANE\" \\; ",
            "select-layout -t \"$source_pane\" \"$layout\""
        ))
        .arg("cryptty-preview")
        .arg(pane)
        .arg(layout.trim())
        .arg(std::env::current_exe()?)
        .args(["image", "--"])
        .args(paths)
        .stdin(Stdio::null())
        .output()
        .context("Open tmux image preview pane")?;
    ensure!(
        output.status.success(),
        "Open tmux image preview pane: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    println!(
        "Opened image preview in pane {}",
        String::from_utf8_lossy(&output.stdout).trim()
    );
    Ok(())
}

pub fn run(paths: &[OsString]) -> Result<()> {
    ensure!(!paths.is_empty(), "Usage: cryptty image FILE [FILE...]");
    let stdout = io::stdout();
    ensure!(stdout.is_terminal(), "Image output requires a terminal");
    let mut size: nix::libc::winsize = unsafe { std::mem::zeroed() };
    let result = unsafe {
        nix::libc::ioctl(stdout.as_raw_fd(), nix::libc::TIOCGWINSZ, &mut size)
    };
    ensure!(
        result == 0 && size.ws_col > 1 && size.ws_row > 1,
        "Cannot determine terminal dimensions"
    );
    let cell = if size.ws_xpixel > 0 && size.ws_ypixel > 0 {
        (
            u32::from(size.ws_xpixel) / u32::from(size.ws_col),
            u32::from(size.ws_ypixel) / u32::from(size.ws_row),
        )
    } else {
        (8, 16)
    };
    let tmux = std::env::var_os("TMUX").is_some_and(|v| !v.is_empty());
    let mut out = stdout.lock();
    for (index, path) in paths.iter().enumerate() {
        let mut bytes = Vec::new();
        File::open(path)
            .with_context(|| format!("Open {}", Path::new(path).display()))?
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        let format = image::guess_format(&bytes)?;
        let image = super::decode_file(&bytes, format)?;
        let seed = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let id = ((seed as u32 ^ std::process::id() ^ index as u32) & 0xffffff)
            .max(1);
        let max_cols = u32::from(size.ws_col - 1).min(296);
        let max_rows = u32::from(size.ws_row - 1).min(296);
        let scale = (max_cols as f64 * f64::from(cell.0.max(1))
            / f64::from(image.width))
        .min(
            max_rows as f64 * f64::from(cell.1.max(1))
                / f64::from(image.height),
        )
        .min(1.0);
        let cols = (f64::from(image.width) * scale / f64::from(cell.0.max(1)))
            .ceil() as u32;
        let rows = (f64::from(image.height) * scale / f64::from(cell.1.max(1)))
            .ceil() as u32;
        let image = if scale < 1.0 {
            let rgba = image::RgbaImage::from_raw(
                image.width,
                image.height,
                image.rgba,
            )
            .context("Invalid image buffer")?;
            let resized = image::imageops::resize(
                &rgba,
                (f64::from(rgba.width()) * scale).ceil().max(1.0) as u32,
                (f64::from(rgba.height()) * scale).ceil().max(1.0) as u32,
                image::imageops::FilterType::Triangle,
            );
            Raster {
                width: resized.width(),
                height: resized.height(),
                rgba: resized.into_raw(),
            }
        } else {
            image
        };
        if tmux && index == 0 {
            enable_tmux_passthrough()?;
        }
        emit(&mut out, &image, id, cols.max(1), rows.max(1), tmux)?;
    }
    out.flush()?;
    Ok(())
}

fn enable_tmux_passthrough() -> Result<()> {
    let pane = std::env::var_os("TMUX_PANE")
        .filter(|pane| !pane.is_empty())
        .context("TMUX_PANE is required to enable image passthrough")?;
    // Preview output must pass through during tmux split redraws.
    let mode = if std::env::var_os("CRYPTTY_IMAGE_PREVIEW").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        "all"
    } else {
        "on"
    };
    let output = Command::new("tmux")
        .args(["set", "-p", "-t"])
        .arg(pane)
        .args(["allow-passthrough", mode])
        .stdin(Stdio::null())
        .output()
        .context("Enable tmux image passthrough for the current pane")?;
    ensure!(
        output.status.success(),
        "Enable tmux image passthrough for the current pane: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

pub fn emit(
    out: &mut impl Write,
    image: &Raster,
    id: u32,
    cols: u32,
    rows: u32,
    tmux: bool,
) -> io::Result<()> {
    let mut png = Vec::new();
    PngEncoder::new_with_quality(
        &mut png,
        CompressionType::Fast,
        FilterType::Adaptive,
    )
    .write_image(
        &image.rgba,
        image.width,
        image.height,
        image::ExtendedColorType::Rgba8,
    )
    .map_err(io::Error::other)?;
    let (format, payload) = if png.len() < image.rgba.len() {
        (100, png.as_slice())
    } else {
        (32, image.rgba.as_slice())
    };
    let data = STANDARD.encode(payload);
    let mut chunks = data.as_bytes().chunks(4096).peekable();
    let mut first = true;
    while let Some(chunk) = chunks.next() {
        let more = u8::from(chunks.peek().is_some());
        let header = if first {
            format!(
                "a=T,U=1,q=2,f={format},s={},v={},\
                 i={id},c={cols},r={rows},m={more}",
                image.width, image.height
            )
        } else {
            format!("m={more}")
        };
        if tmux {
            out.write_all(b"\x1bPtmux;\x1b")?
        }
        write!(out, "\x1b_G{header};")?;
        out.write_all(chunk)?;
        if tmux {
            out.write_all(b"\x1b")?
        }
        out.write_all(b"\x1b\\")?;
        if tmux {
            out.write_all(b"\x1b\\")?
        }
        first = false;
    }
    write!(
        out,
        "\r\x1b[38;2;{};{};{}m",
        id >> 16 & 255,
        id >> 8 & 255,
        id & 255
    )?;
    for row in 0..rows {
        for col in 0..cols {
            write!(
                out,
                "{}{}{}",
                super::MARKER,
                DIACRITICS[row as usize],
                DIACRITICS[col as usize]
            )?;
        }
        out.write_all(b"\r\n")?;
    }
    out.write_all(b"\x1b[39m")
}
