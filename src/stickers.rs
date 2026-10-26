//! Rasterizes animated (TGS, gzipped Lottie) and video (WebM) stickers into
//! frames GPUI can play. Runs on a background thread.

use std::{
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use gpui_kit::{RenderImage, swap_rgba_pa_to_bgra};
use image::{Delay, Frame, RgbaImage};
use rasterlottie::{Animation, RenderConfig, Renderer, Rgba8};

/// Playback rate; stickers are authored at 30 or 60 fps.
const FPS: f32 = 30.;
/// Telegram caps stickers at 3 seconds.
const MAX_FRAMES: usize = 90;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Lottie,
    Video,
}

pub struct Spec {
    pub kind: Kind,
    pub path: PathBuf,
    /// Output size in pixels.
    pub width: u32,
    pub height: u32,
    /// False renders only the first frame (sticker picker).
    pub animate: bool,
}

/// Decoded frames, each its own image: the app picks the frame to show on
/// its own clock (see `Messenger::animation`) instead of letting GPUI
/// redraw the window at the display's refresh rate for multi-frame images.
pub struct Frames {
    pub images: Vec<Arc<RenderImage>>,
    pub delay: Duration,
}

pub fn decode(spec: &Spec) -> Result<Frames> {
    let frames = match spec.kind {
        Kind::Lottie => lottie(spec)?,
        Kind::Video => video(spec)?,
    };
    ensure!(!frames.is_empty(), "sticker has no frames");
    let delay = frames
        .first()
        .map_or(Duration::from_millis(33), |frame| frame.delay().into())
        .max(Duration::from_millis(16));
    Ok(Frames {
        images: frames
            .into_iter()
            .map(|frame| Arc::new(RenderImage::new(vec![frame])))
            .collect(),
        delay,
    })
}

fn lottie(spec: &Spec) -> Result<Vec<Frame>> {
    let mut json = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&spec.path)?)
        .read_to_string(&mut json)
        .context("not a gzipped Lottie file")?;
    let animation = Animation::from_json_str(&json)?;
    let scale = spec.width as f32 / animation.width.max(1) as f32;
    let config = RenderConfig::new(Rgba8::TRANSPARENT, scale);
    let prepared = Renderer::default().prepare(&animation)?;
    let source_fps = animation.frame_rate.max(1.);
    let step = (source_fps / FPS).max(1.);
    let fps = source_fps / step;
    let delay = Delay::from_numer_denom_ms(1000, fps.round().max(1.) as u32);
    let mut frames = Vec::new();
    let mut time = animation.in_point;
    while time < animation.out_point && frames.len() < MAX_FRAMES {
        let mut raster = prepared.render_frame(time, config)?;
        // tiny-skia renders premultiplied RGBA; GPUI wants straight BGRA.
        for pixel in raster.pixels.as_chunks_mut::<4>().0 {
            swap_rgba_pa_to_bgra(pixel);
        }
        let buffer = RgbaImage::from_raw(raster.width, raster.height, raster.pixels)
            .context("frame size mismatch")?;
        frames.push(Frame::from_parts(buffer, 0, 0, delay));
        if !spec.animate {
            break;
        }
        time += step;
    }
    Ok(frames)
}

fn video(spec: &Spec) -> Result<Vec<Frame>> {
    let ffmpeg = crate::media::ffmpeg().context("ffmpeg is not installed")?;
    let (width, height) = (spec.width, spec.height);
    let mut command = Command::new(ffmpeg);
    // libvpx decodes the VP9 alpha channel; the built-in decoder drops it.
    command
        .args(["-v", "error", "-c:v", "libvpx-vp9", "-i"])
        .arg(&spec.path)
        .args([
            "-vf",
            &format!("fps={FPS},scale={width}:{height}"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
        ]);
    let wanted = if spec.animate { MAX_FRAMES } else { 1 };
    command.args(["-frames:v", &wanted.to_string()]);
    let mut child = command
        .arg("-")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdout = child.stdout.take().context("no ffmpeg output")?;
    // Read frame by frame into each frame's own buffer, rather than
    // collecting ffmpeg's whole output and copying it out again.
    let frame_len = (width * height * 4) as usize;
    let delay = Delay::from_numer_denom_ms(1000, FPS as u32);
    let mut frames = Vec::new();
    while frames.len() < wanted {
        let mut pixels = vec![0u8; frame_len];
        if stdout.read_exact(&mut pixels).is_err() {
            break;
        }
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        let buffer = RgbaImage::from_raw(width, height, pixels).context("frame size mismatch")?;
        frames.push(Frame::from_parts(buffer, 0, 0, delay));
    }
    drop(stdout);
    let status = child.wait()?;
    if frames.is_empty() && !status.success() {
        bail!("ffmpeg failed to decode the sticker");
    }
    Ok(frames)
}
