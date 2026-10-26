//! In-app playback: voice messages, audio files and round video messages.
//! Audio goes through AVAudioPlayer; formats macOS can't play (Ogg/Opus
//! voice notes) are rewritten as CAF first (see `opus`). Round videos are
//! decoded by AVFoundation, paced by the audio clock.

use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
};

use anyhow::{Context, Result, bail};
use gpui_kit::RenderImage;
use image::{Delay, Frame, RgbaImage};
use objc2::{AnyThread, rc::Retained};
use objc2_avf_audio::AVAudioPlayer;
use objc2_foundation::{NSString, NSURL};

pub struct Player {
    player: Retained<AVAudioPlayer>,
}

impl Player {
    pub fn open(path: &Path) -> Result<Self> {
        let path = path.to_str().context("non-UTF-8 path")?;
        // SAFETY: Plain AVAudioPlayer calls on the main thread with a file URL.
        unsafe {
            let url = NSURL::fileURLWithPath(&NSString::from_str(path));
            let player = AVAudioPlayer::initWithContentsOfURL_error(AVAudioPlayer::alloc(), &url)
                .map_err(|error| anyhow::anyhow!("{}", error.localizedDescription()))?;
            player.prepareToPlay();
            Ok(Self { player })
        }
    }

    pub fn play(&self) {
        // SAFETY: See `open`.
        unsafe {
            self.player.play();
        }
    }

    pub fn pause(&self) {
        // SAFETY: See `open`.
        unsafe { self.player.pause() }
    }

    pub fn is_playing(&self) -> bool {
        // SAFETY: See `open`.
        unsafe { self.player.isPlaying() }
    }

    pub fn time(&self) -> f64 {
        // SAFETY: See `open`.
        unsafe { self.player.currentTime() }
    }

    pub fn duration(&self) -> f64 {
        // SAFETY: See `open`.
        unsafe { self.player.duration() }
    }

    pub fn seek(&self, seconds: f64) {
        // SAFETY: See `open`.
        unsafe {
            self.player
                .setCurrentTime(seconds.clamp(0., self.duration()))
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // SAFETY: See `open`.
        unsafe { self.player.stop() }
    }
}

/// Returns a file AVAudioPlayer can open: the original when macOS supports
/// its format, otherwise (Ogg Opus voice notes) a CAF copy made once in the
/// cache.
pub fn playable(path: &Path, cache: &Path) -> Result<PathBuf> {
    let native = [
        "mp3", "m4a", "aac", "wav", "aif", "aiff", "caf", "mp4", "m4b", "flac",
    ];
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if native.contains(&extension.as_str()) {
        return Ok(path.to_owned());
    }
    // Voice notes are Ogg Opus, whatever their extension.
    let mut magic = [0u8; 4];
    let ogg = std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_ok()
        && &magic == b"OggS";
    if !ogg {
        bail!("this audio format isn't supported");
    }
    std::fs::create_dir_all(cache)?;
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("audio");
    let output = cache.join(format!("{name}.caf"));
    if !output.exists() {
        crate::opus::ogg_to_caf(path, &output)?;
    }
    Ok(output)
}

/// Where converted audio is cached between plays.
pub fn audio_cache() -> PathBuf {
    std::env::temp_dir().join("kite-audio")
}

/// Deletes converted audio not played for a week, so the cache doesn't grow
/// forever. Runs off the main thread at startup.
pub fn prune_audio_cache() {
    let week = std::time::Duration::from_secs(7 * 24 * 60 * 60);
    let Ok(entries) = std::fs::read_dir(audio_cache()) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.accessed().or_else(|_| m.modified()))
            .ok()
            .and_then(|time| time.elapsed().ok())
            .is_some_and(|age| age > week);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Frames of a video decoded on a background thread by AVFoundation. The
/// channel holds only a few frames, so decoding stays just ahead of
/// playback; dropping the stream stops it.
pub struct VideoStream {
    frames: Receiver<(f64, RenderImage)>,
    next: Option<(f64, RenderImage)>,
}

/// Round videos animate at their own frame rate; playback checks for a new
/// frame this often.
pub const VIDEO_FPS: u32 = 30;

impl VideoStream {
    /// Decodes `path` (a square round video) at `size` pixels square.
    pub fn open(path: &Path, size: u32) -> Result<Self> {
        let path = path.to_str().context("non-UTF-8 path")?.to_owned();
        let (sender, frames) = sync_channel(4);
        std::thread::Builder::new()
            .name("video-frames".into())
            .spawn(move || {
                objc2::rc::autoreleasepool(|_| {
                    let _ = decode_video(&path, size, &sender);
                });
            })?;
        Ok(Self { frames, next: None })
    }

    /// The newest frame due at `time` seconds, if a new one is due.
    pub fn frame_at(&mut self, time: f64) -> Option<RenderImage> {
        let mut due = None;
        loop {
            if self.next.is_none() {
                match self.frames.try_recv() {
                    Ok(frame) => self.next = Some(frame),
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
            }
            match &self.next {
                Some((at, _)) if *at <= time => due = self.next.take().map(|(_, f)| f),
                _ => break,
            }
        }
        due
    }
}

/// Reads the video track's frames as BGRA at `size` square and sends them
/// with their presentation times, until done or the receiver is dropped.
fn decode_video(path: &str, size: u32, sender: &SyncSender<(f64, RenderImage)>) -> Result<()> {
    use objc2_av_foundation::{AVAsset, AVAssetReader, AVAssetReaderTrackOutput, AVMediaTypeVideo};
    use objc2_core_video::{
        CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
        CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
        CVPixelBufferUnlockBaseAddress, kCVPixelFormatType_32BGRA,
    };
    use objc2_foundation::{NSDictionary, NSNumber};

    // SAFETY: AVFoundation reading on this thread only; every object is
    // created and used here, and pixel memory is read between lock/unlock.
    unsafe {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let asset = AVAsset::assetWithURL(&url);
        let media = AVMediaTypeVideo.context("no video media type")?;
        #[allow(deprecated)] // The async loader needs a run loop this thread lacks.
        let track = asset
            .tracksWithMediaType(media)
            .firstObject()
            .context("no video track")?;
        // Decoded straight to GPUI's pixel layout and size.
        let keys = [
            NSString::from_str("PixelFormatType"),
            NSString::from_str("Width"),
            NSString::from_str("Height"),
        ];
        let values = [
            NSNumber::new_u32(kCVPixelFormatType_32BGRA),
            NSNumber::new_u32(size),
            NSNumber::new_u32(size),
        ];
        let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
        let value_refs: Vec<&objc2::runtime::AnyObject> = values
            .iter()
            .map(|v| &***v as &objc2::runtime::AnyObject)
            .collect();
        let settings = NSDictionary::from_slices(&key_refs, &value_refs);
        let output = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(
            &track,
            Some(&settings),
        );
        let reader = AVAssetReader::assetReaderWithAsset_error(&asset)
            .map_err(|e| anyhow::anyhow!("{}", e.localizedDescription()))?;
        reader.addOutput(&output);
        if !reader.startReading() {
            bail!("could not start reading the video");
        }
        while let Some(sample) = output.copyNextSampleBuffer() {
            let time = sample.presentation_time_stamp().seconds();
            let Some(buffer) = sample.image_buffer() else {
                continue;
            };
            if CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags::ReadOnly) != 0 {
                continue;
            }
            let (width, height) = (
                CVPixelBufferGetWidth(&buffer),
                CVPixelBufferGetHeight(&buffer),
            );
            let stride = CVPixelBufferGetBytesPerRow(&buffer);
            let base = CVPixelBufferGetBaseAddress(&buffer).cast::<u8>();
            let mut pixels = Vec::with_capacity(width * height * 4);
            if !base.is_null() {
                for row in 0..height {
                    pixels.extend_from_slice(std::slice::from_raw_parts(
                        base.add(row * stride),
                        width * 4,
                    ));
                }
            }
            CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags::ReadOnly);
            let Some(image) = RgbaImage::from_raw(width as u32, height as u32, pixels) else {
                continue;
            };
            let frame = Frame::from_parts(image, 0, 0, Delay::from_numer_denom_ms(0, 1));
            if sender.send((time, RenderImage::new(vec![frame]))).is_err() {
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The demo round video decodes to timed, non-blank square frames.
    #[test]
    fn demo_round_video() -> Result<()> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/demo/round.mp4");
        let (sender, frames) = sync_channel(1024);
        objc2::rc::autoreleasepool(|_| decode_video(path, 320, &sender))?;
        drop(sender);
        let frames: Vec<_> = frames.iter().collect();
        // A 5 second clip.
        anyhow::ensure!(frames.len() >= 100, "only {} frames", frames.len());
        anyhow::ensure!(
            frames.windows(2).all(|w| w[1].0 > w[0].0),
            "times out of order"
        );
        let last = frames.last().map_or(0., |f| f.0);
        anyhow::ensure!((4.5..5.1).contains(&last), "ends at {last}s");
        let (_, image) = &frames[frames.len() / 2];
        let size = image.size(0);
        anyhow::ensure!(size.width.0 == 320 && size.height.0 == 320, "size {size:?}");
        let bytes = image.as_bytes(0).context("no pixels")?;
        let lit = bytes.iter().filter(|&&b| b > 16).count();
        anyhow::ensure!(lit > bytes.len() / 4, "frame is mostly black");
        Ok(())
    }
}
