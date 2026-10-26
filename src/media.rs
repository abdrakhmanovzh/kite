//! In-app playback: voice messages, audio files and round video messages.
//! Audio goes through AVAudioPlayer; formats macOS can't play (Ogg/Opus
//! voice notes) are converted with ffmpeg first. Round videos stream frames
//! from ffmpeg, paced by the audio clock.

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{Receiver, TryRecvError, sync_channel},
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
/// its format, otherwise an AAC copy converted once into the cache.
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
    std::fs::create_dir_all(cache)?;
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("audio");
    let output = cache.join(format!("{name}.m4a"));
    if output.exists() {
        return Ok(output);
    }
    let partial = cache.join(format!("{name}.partial.m4a"));
    let status = Command::new(ffmpeg().context("ffmpeg is not installed")?)
        .args(["-v", "error", "-y", "-i"])
        .arg(path)
        .args(["-vn", "-c:a", "aac", "-b:a", "96k"])
        .arg(&partial)
        .stdin(Stdio::null())
        .status()?;
    if !status.success() {
        bail!("could not convert audio");
    }
    std::fs::rename(&partial, &output)?;
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

/// Frames of a video decoded on the fly by ffmpeg. The channel holds only a
/// few frames, so ffmpeg stays just ahead of playback.
pub struct VideoStream {
    frames: Receiver<RenderImage>,
    next: Option<(f64, RenderImage)>,
    index: u32,
    child: Child,
}

pub const VIDEO_FPS: u32 = 30;

impl VideoStream {
    pub fn open(path: &Path, size: u32) -> Result<Self> {
        let mut child = Command::new(ffmpeg().context("ffmpeg is not installed")?)
            .args(["-v", "error", "-i"])
            .arg(path)
            .args([
                "-vf",
                &format!("fps={VIDEO_FPS},scale={size}:{size}:force_original_aspect_ratio=increase,crop={size}:{size}"),
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdout = child.stdout.take().context("no ffmpeg output")?;
        let (sender, frames) = sync_channel(4);
        let frame_len = (size * size * 4) as usize;
        std::thread::Builder::new()
            .name("video-frames".into())
            .spawn(move || {
                loop {
                    // Read straight into the frame's own buffer: no copy.
                    let mut pixels = vec![0u8; frame_len];
                    if stdout.read_exact(&mut pixels).is_err() {
                        break;
                    }
                    for pixel in pixels.as_chunks_mut::<4>().0 {
                        pixel.swap(0, 2);
                    }
                    let Some(image) = RgbaImage::from_raw(size, size, pixels) else {
                        break;
                    };
                    let frame = Frame::from_parts(image, 0, 0, Delay::from_numer_denom_ms(0, 1));
                    if sender.send(RenderImage::new(vec![frame])).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            frames,
            next: None,
            index: 0,
            child,
        })
    }

    /// The newest frame due at `time` seconds, if a new one is due.
    pub fn frame_at(&mut self, time: f64) -> Option<RenderImage> {
        let mut due = None;
        loop {
            if self.next.is_none() {
                match self.frames.try_recv() {
                    Ok(frame) => {
                        let at = f64::from(self.index) / f64::from(VIDEO_FPS);
                        self.index += 1;
                        self.next = Some((at, frame));
                    }
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

impl Drop for VideoStream {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Apps launched from Finder don't inherit the shell's PATH, so check the
/// usual install locations first.
pub fn ffmpeg() -> Option<PathBuf> {
    ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"]
        .iter()
        .map(PathBuf::from)
        .find(|path| path.exists())
        .or_else(|| {
            std::env::var_os("PATH")?
                .to_str()?
                .split(':')
                .map(|dir| Path::new(dir).join("ffmpeg"))
                .find(|path| path.exists())
        })
}
