//! Rasterizes animated (TGS, gzipped Lottie) and video (WebM) stickers into
//! frames GPUI can play. Runs on a background thread.

use std::{io::Read, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
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
    let data = std::fs::read(&spec.path)?;
    let packets = crate::webm::packets(&data)?;
    ensure!(!packets.is_empty(), "no video frames");
    // Every frame must be decoded (VP9 frames build on each other), but
    // only every `step`-th is kept, to play at about FPS.
    let span = packets.last().map_or(0., |p| p.time_ms) - packets[0].time_ms;
    let source_fps = if span > 0. {
        (packets.len() - 1) as f32 * 1000. / span as f32
    } else {
        FPS
    };
    let step = (source_fps / FPS).round().max(1.) as usize;
    let delay = Delay::from_numer_denom_ms((1000. * step as f32 / source_fps).round() as u32, 1);
    let mut color = crate::webm::Decoder::new()?;
    let mut alpha = crate::webm::Decoder::new()?;
    let mut frames = Vec::new();
    for (index, packet) in packets.iter().enumerate() {
        let picture = color.decode(&packet.color)?;
        let mask = match &packet.alpha {
            Some(data) => alpha.decode(data)?,
            None => None,
        };
        if index % step != 0 {
            continue;
        }
        let Some(picture) = picture else {
            continue;
        };
        let full = RgbaImage::from_raw(
            picture.width as u32,
            picture.height as u32,
            picture.to_bgra(mask.as_ref()),
        )
        .context("frame size mismatch")?;
        frames.push(Frame::from_parts(scaled(full, spec), 0, 0, delay));
        if !spec.animate || frames.len() >= MAX_FRAMES {
            break;
        }
    }
    Ok(frames)
}

/// Resizes a straight-alpha BGRA frame to the spec's size, premultiplying
/// around the resize so transparent pixels' colors don't bleed into edges.
fn scaled(mut image: RgbaImage, spec: &Spec) -> RgbaImage {
    if image.dimensions() == (spec.width, spec.height) {
        return image;
    }
    for pixel in image.pixels_mut() {
        let a = u16::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = ((u16::from(*channel) * a + 127) / 255) as u8;
        }
    }
    let mut image = image::imageops::resize(
        &image,
        spec.width,
        spec.height,
        image::imageops::FilterType::Triangle,
    );
    for pixel in image.pixels_mut() {
        let a = u16::from(pixel[3]);
        if let Some(divisor) = std::num::NonZeroU16::new(a) {
            for channel in &mut pixel.0[..3] {
                *channel = ((u16::from(*channel) * 255 + a / 2) / divisor).min(255) as u8;
            }
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A VP9 sticker with transparency: clear corners, an opaque blue box.
    #[test]
    fn video_sticker_with_alpha() -> Result<()> {
        let spec = Spec {
            kind: Kind::Video,
            path: concat!(env!("CARGO_MANIFEST_DIR"), "/assets/demo/sticker-box.webm").into(),
            width: 240,
            height: 240,
            animate: true,
        };
        let frames = video(&spec)?;
        // Two seconds at 30 fps.
        ensure!((55..=61).contains(&frames.len()), "{} frames", frames.len());
        let first = frames[0].buffer();
        let at = |x, y| first.get_pixel(x, y).0;
        ensure!(at(4, 4)[3] == 0, "corner isn't transparent: {:?}", at(4, 4));
        let [b, g, r, a] = at(120, 120);
        ensure!(
            a > 240 && b > 230 && r < 110,
            "box pixel {:?}",
            [b, g, r, a]
        );
        Ok(())
    }
}
