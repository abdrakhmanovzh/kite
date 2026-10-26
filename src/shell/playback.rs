//! Playing voice messages, audio files and round videos inside the app.

use std::{path::PathBuf, sync::Arc, time::Duration};

use gpui_kit::{prelude::*, *};
use tdlib_rs::{enums::MessageContent as C, types::Message};

use super::Messenger;
use crate::{media, ui};

pub(super) struct Playback {
    pub chat_id: i64,
    pub message_id: i64,
    player: media::Player,
    video: Option<media::VideoStream>,
    /// The round video's current frame.
    pub frame: Option<Arc<RenderImage>>,
    /// Paused by the user (as opposed to finished).
    paused: bool,
    _tick: Task<()>,
}

impl Playback {
    pub fn is_playing(&self) -> bool {
        !self.paused
    }

    pub fn time(&self) -> f64 {
        self.player.time()
    }

    pub fn progress(&self) -> f32 {
        let duration = self.player.duration();
        if duration > 0. {
            (self.player.time() / duration) as f32
        } else {
            0.
        }
    }
}

/// The playable file of a message, and whether it is a round video.
fn playable_file(message: &Message) -> Option<(&tdlib_rs::types::File, bool)> {
    match &message.content {
        C::MessageVoiceNote(m) => Some((&m.voice_note.voice, false)),
        C::MessageAudio(m) => Some((&m.audio.audio, false)),
        C::MessageVideoNote(m) => Some((&m.video_note.video, true)),
        _ => None,
    }
}

impl Messenger {
    pub(super) fn playback_of(&self, chat_id: i64, message_id: i64) -> Option<&Playback> {
        self.playback
            .as_ref()
            .filter(|p| p.chat_id == chat_id && p.message_id == message_id)
    }

    /// Plays, pauses or resumes a voice message, audio file or round video.
    pub(super) fn toggle_play(&mut self, chat_id: i64, message_id: i64, cx: &mut Context<Self>) {
        if let Some(playback) = self
            .playback
            .as_mut()
            .filter(|p| p.chat_id == chat_id && p.message_id == message_id)
        {
            if playback.paused {
                playback.player.play();
                playback.paused = false;
            } else {
                playback.player.pause();
                playback.paused = true;
            }
            cx.notify();
            return;
        }
        self.stop_playback(cx);
        let Some((file, _)) = self
            .find_message(chat_id, message_id)
            .and_then(playable_file)
        else {
            return;
        };
        let file = self.file(file).clone();
        if file.local.is_downloading_completed {
            self.start_playback(chat_id, message_id, cx);
        } else {
            // Start once the download finishes.
            self.pending_play = Some((chat_id, message_id, file.id));
            self.request_file(&file, 32, false);
        }
        cx.notify();
    }

    /// Called when a download completes; starts a playback waiting on it.
    pub(super) fn resume_pending_play(&mut self, file_id: i32, cx: &mut Context<Self>) {
        if let Some((chat_id, message_id, id)) = self.pending_play
            && id == file_id
        {
            self.pending_play = None;
            self.start_playback(chat_id, message_id, cx);
        }
    }

    fn start_playback(&mut self, chat_id: i64, message_id: i64, cx: &mut Context<Self>) {
        let Some((file, round)) = self
            .find_message(chat_id, message_id)
            .and_then(playable_file)
        else {
            return;
        };
        let path = PathBuf::from(&self.file(file).local.path);
        let cache = media::audio_cache();
        self.pending_play = Some((chat_id, message_id, -1));
        // Conversion (Ogg/Opus voice notes) runs off the main thread.
        let source = path.clone();
        let converted = cx
            .background_executor()
            .spawn(async move { media::playable(&source, &cache) });
        cx.spawn(async move |this, cx| {
            let audio = converted.await;
            let _ = this.update(cx, |this, cx| {
                // Another message was started meanwhile.
                if this.pending_play != Some((chat_id, message_id, -1)) {
                    return;
                }
                this.pending_play = None;
                let started = audio.and_then(|audio| {
                    let player = media::Player::open(&audio)?;
                    let video = if round {
                        Some(media::VideoStream::open(&path, 320)?)
                    } else {
                        None
                    };
                    Ok((player, video))
                });
                match started {
                    Ok((player, video)) => {
                        player.play();
                        // Video needs every frame; audio only moves a progress bar.
                        let interval = if video.is_some() {
                            Duration::from_millis(1000 / u64::from(media::VIDEO_FPS))
                        } else {
                            Duration::from_millis(100)
                        };
                        let tick = cx.spawn(async move |this, cx| {
                            loop {
                                smol::Timer::after(interval).await;
                                if this.update(cx, |this, cx| this.tick_playback(cx)).is_err() {
                                    break;
                                }
                            }
                        });
                        this.playback = Some(Playback {
                            chat_id,
                            message_id,
                            player,
                            video,
                            frame: None,
                            paused: false,
                            _tick: tick,
                        });
                    }
                    Err(error) => this.error = Some(format!("Can't play this: {error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn tick_playback(&mut self, cx: &mut Context<Self>) {
        let Some(playback) = &mut self.playback else {
            return;
        };
        if playback.paused {
            return;
        }
        if !playback.player.is_playing() {
            // Reached the end.
            self.stop_playback(cx);
            cx.notify();
            return;
        }
        let time = playback.player.time();
        if let Some(video) = &mut playback.video
            && let Some(frame) = video.frame_at(time)
        {
            // Each frame is a new texture; free the one it replaces.
            if let Some(old) = playback.frame.replace(Arc::new(frame)) {
                cx.drop_image(old, None);
            }
        }
        self.redraw_conversation(cx);
    }

    /// Ends playback and frees the last video frame's texture.
    pub(super) fn stop_playback(&mut self, cx: &mut App) {
        if let Some(frame) = self.playback.take().and_then(|p| p.frame) {
            cx.drop_image(frame, None);
        }
    }

    pub(super) fn seek(
        &mut self,
        chat_id: i64,
        message_id: i64,
        fraction: f32,
        cx: &mut Context<Self>,
    ) {
        if let Some(playback) = self
            .playback
            .as_mut()
            .filter(|p| p.chat_id == chat_id && p.message_id == message_id && p.video.is_none())
        {
            let duration = playback.player.duration();
            playback.player.seek(duration * f64::from(fraction));
            cx.notify();
        }
    }

    /// Whether a playback for this message is waiting on download or conversion.
    pub(super) fn play_pending(&self, chat_id: i64, message_id: i64) -> bool {
        self.pending_play
            .is_some_and(|(c, m, _)| c == chat_id && m == message_id)
    }
}

/// Telegram's voice waveform: 5-bit samples packed little-endian, reduced
/// to `bars` peaks scaled to 0..=1.
pub(super) fn waveform(data: &str, bars: usize) -> Vec<f32> {
    let bytes = ui::base64(data);
    let samples: Vec<u8> = (0..bytes.len() * 8 / 5)
        .map(|i| {
            let bit = i * 5;
            let low = u16::from(bytes[bit / 8]);
            let high = u16::from(bytes.get(bit / 8 + 1).copied().unwrap_or(0));
            (((low | (high << 8)) >> (bit % 8)) & 0x1f) as u8
        })
        .collect();
    if samples.is_empty() {
        return vec![0.3; bars];
    }
    (0..bars)
        .map(|bar| {
            let start = bar * samples.len() / bars;
            let end = ((bar + 1) * samples.len() / bars).max(start + 1);
            let peak = samples[start..end.min(samples.len())]
                .iter()
                .copied()
                .max()
                .unwrap_or(0);
            f32::from(peak) / 31.
        })
        .collect()
}
