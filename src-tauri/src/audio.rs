use std::collections::VecDeque;
use std::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rodio::{Decoder, DeviceSinkBuilder, Player, Source};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::notify::QuotaKind;

const MAX_PLAYBACK: Duration = Duration::from_secs(10);
const PREVIEW_REPLY_TIMEOUT: Duration = Duration::from_secs(5);
const WORKER_POLL: Duration = Duration::from_millis(25);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioError {
    UnsupportedFormat,
    PathMissing,
    Undecodable(String),
    DeviceUnavailable,
    Busy,
}

impl AudioError {
    pub fn key(&self) -> &'static str {
        match self {
            Self::UnsupportedFormat => "sound_unsupported_format",
            Self::PathMissing => "sound_path_missing",
            Self::Undecodable(_) => "sound_undecodable",
            Self::DeviceUnavailable => "sound_device_unavailable",
            Self::Busy => "sound_audio_busy",
        }
    }
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Undecodable(detail) => write!(f, "{}: {detail}", self.key()),
            _ => f.write_str(self.key()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeInfo {
    pub container: &'static str,
}

pub fn probe_audio(path: &Path) -> Result<ProbeInfo, AudioError> {
    let container = supported_container(path)?;
    if !path.is_file() {
        return Err(AudioError::PathMissing);
    }
    let file = File::open(path).map_err(|_| AudioError::PathMissing)?;
    let mut decoder =
        Decoder::try_from(file).map_err(|error| AudioError::Undecodable(error.to_string()))?;
    let mut decoded = 0usize;
    for sample in decoder
        .by_ref()
        .take_duration(Duration::from_secs(1))
        .take(8_192)
    {
        let _ = sample;
        decoded += 1;
    }
    if decoded == 0 {
        return Err(AudioError::Undecodable("empty audio stream".into()));
    }
    Ok(ProbeInfo { container })
}

fn supported_container(path: &Path) -> Result<&'static str, AudioError> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mp3") => Ok("mp3"),
        Some("aac") => Ok("aac-adts"),
        Some("m4a") => Ok("m4a"),
        Some("wav") => Ok("wav"),
        _ => Err(AudioError::UnsupportedFormat),
    }
}

fn load_limited_source(path: &Path) -> Result<impl Source + Send + 'static, AudioError> {
    if !path.is_file() {
        return Err(AudioError::PathMissing);
    }
    let file = File::open(path).map_err(|_| AudioError::PathMissing)?;
    let source =
        Decoder::try_from(file).map_err(|error| AudioError::Undecodable(error.to_string()))?;
    Ok(source.take_duration(MAX_PLAYBACK))
}

enum AudioCommand {
    EnqueueNotification {
        path: PathBuf,
        kind: QuotaKind,
    },
    Preview {
        path: PathBuf,
        kind: QuotaKind,
        reply: mpsc::SyncSender<Result<(), AudioError>>,
    },
    StopPreview,
    Shutdown,
}

struct WorkerShared {
    sender: mpsc::Sender<AudioCommand>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Clone)]
pub struct AudioHandle(Arc<WorkerShared>);

impl AudioHandle {
    pub fn enqueue_notification(&self, path: PathBuf, kind: QuotaKind) {
        let _ = self
            .0
            .sender
            .send(AudioCommand::EnqueueNotification { path, kind });
    }

    pub fn preview(&self, path: PathBuf, kind: QuotaKind) -> Result<(), AudioError> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.0
            .sender
            .send(AudioCommand::Preview {
                path,
                kind,
                reply: reply_tx,
            })
            .map_err(|_| AudioError::Busy)?;
        reply_rx
            .recv_timeout(PREVIEW_REPLY_TIMEOUT)
            .map_err(|_| AudioError::Busy)?
    }

    pub fn stop_preview(&self) {
        let _ = self.0.sender.send(AudioCommand::StopPreview);
    }

    pub fn shutdown(&self) {
        let _ = self.0.sender.send(AudioCommand::Shutdown);
        if let Ok(mut worker) = self.0.worker.lock() {
            if let Some(worker) = worker.take() {
                let _ = worker.join();
            }
        }
    }
}

#[derive(Clone)]
struct AudioJob {
    path: PathBuf,
    kind: Option<QuotaKind>,
    preview: bool,
}

#[derive(Default)]
struct PlaybackQueue {
    notifications: VecDeque<AudioJob>,
    previews: VecDeque<AudioJob>,
}

impl PlaybackQueue {
    fn push_notification(&mut self, path: PathBuf, kind: QuotaKind) {
        self.notifications.push_back(AudioJob {
            path,
            kind: Some(kind),
            preview: false,
        });
    }

    fn push_preview(&mut self, path: PathBuf, kind: QuotaKind) {
        self.previews.push_back(AudioJob {
            path,
            kind: Some(kind),
            preview: true,
        });
    }

    fn pop_next(&mut self) -> Option<AudioJob> {
        self.notifications
            .pop_front()
            .or_else(|| self.previews.pop_front())
    }

    fn clear_previews(&mut self) {
        self.previews.clear();
    }
}

struct CurrentPlayback {
    player: Player,
    job: AudioJob,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlaybackErrorEvent {
    kind: Option<QuotaKind>,
    error: String,
}

pub fn spawn_worker(app: AppHandle) -> Result<AudioHandle, String> {
    let (sender, receiver) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("codex-usage-audio".into())
        .spawn(move || run_worker(app, receiver))
        .map_err(|error| error.to_string())?;
    Ok(AudioHandle(Arc::new(WorkerShared {
        sender,
        worker: Mutex::new(Some(worker)),
    })))
}

fn run_worker(app: AppHandle, receiver: mpsc::Receiver<AudioCommand>) {
    let mut queue = PlaybackQueue::default();
    let mut sink = None;
    let mut current: Option<CurrentPlayback> = None;
    let mut shutdown = false;

    while !shutdown {
        if current
            .as_ref()
            .is_some_and(|playing| playing.player.empty())
        {
            let ended = current.take().expect("current playback was checked above");
            if ended.job.preview {
                let _ = app.emit("sound_preview_finished", ());
            }
        }

        if current.is_none() {
            if let Some(job) = queue.pop_next() {
                let source = match load_limited_source(&job.path) {
                    Ok(source) => source,
                    Err(error) => {
                        report_playback_error(&app, &job, error);
                        continue;
                    }
                };
                if sink.is_none() {
                    match DeviceSinkBuilder::open_default_sink() {
                        Ok(device) => sink = Some(device),
                        Err(_) => {
                            report_playback_error(&app, &job, AudioError::DeviceUnavailable);
                            continue;
                        }
                    }
                }
                let player =
                    Player::connect_new(sink.as_ref().expect("audio device was opened").mixer());
                player.append(source);
                current = Some(CurrentPlayback { player, job });
                continue;
            }
        }

        match receiver.recv_timeout(WORKER_POLL) {
            Ok(AudioCommand::EnqueueNotification { path, kind }) => {
                queue.push_notification(path, kind)
            }
            Ok(AudioCommand::Preview { path, kind, reply }) => {
                let validation = probe_audio(&path).and_then(|_| {
                    if sink.is_none() {
                        sink = DeviceSinkBuilder::open_default_sink().ok();
                    }
                    if sink.is_some() {
                        Ok(())
                    } else {
                        Err(AudioError::DeviceUnavailable)
                    }
                });
                match validation {
                    Ok(()) => {
                        queue.push_preview(path, kind);
                        let _ = reply.send(Ok(()));
                    }
                    Err(error) => {
                        let _ = reply.send(Err(error));
                    }
                }
            }
            Ok(AudioCommand::StopPreview) => {
                queue.clear_previews();
                if current.as_ref().is_some_and(|playing| playing.job.preview) {
                    if let Some(playing) = current.take() {
                        playing.player.stop();
                    }
                    let _ = app.emit("sound_preview_finished", ());
                }
            }
            Ok(AudioCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                shutdown = true;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }

    queue.notifications.clear();
    queue.previews.clear();
    if let Some(playing) = current.take() {
        playing.player.stop();
    }
    drop(sink);
}

fn report_playback_error(app: &AppHandle, job: &AudioJob, error: AudioError) {
    let _ = app.emit(
        "sound_playback_error",
        PlaybackErrorEvent {
            kind: job.kind,
            error: error.to_string(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_filter_is_case_insensitive_and_distinguishes_aac_containers() {
        assert_eq!(supported_container(Path::new("tone.MP3")).unwrap(), "mp3");
        assert_eq!(
            supported_container(Path::new("tone.AAC")).unwrap(),
            "aac-adts"
        );
        assert_eq!(supported_container(Path::new("tone.M4A")).unwrap(), "m4a");
        assert_eq!(supported_container(Path::new("tone.WAV")).unwrap(), "wav");
        assert_eq!(
            supported_container(Path::new("tone.ogg")),
            Err(AudioError::UnsupportedFormat)
        );
    }

    #[test]
    fn notification_jobs_run_before_previews() {
        let mut queue = PlaybackQueue::default();
        queue.push_preview("preview.wav".into(), QuotaKind::FiveHour);
        queue.push_notification("first.wav".into(), QuotaKind::FiveHour);
        queue.push_notification("second.wav".into(), QuotaKind::Week);
        assert_eq!(queue.pop_next().unwrap().path, PathBuf::from("first.wav"));
        assert_eq!(queue.pop_next().unwrap().path, PathBuf::from("second.wav"));
        assert!(queue.pop_next().unwrap().preview);
    }

    #[test]
    fn stopping_previews_does_not_remove_notification_jobs() {
        let mut queue = PlaybackQueue::default();
        queue.push_notification("alert.wav".into(), QuotaKind::Week);
        queue.push_preview("preview.wav".into(), QuotaKind::FiveHour);
        queue.clear_previews();
        assert_eq!(queue.pop_next().unwrap().path, PathBuf::from("alert.wav"));
        assert!(queue.pop_next().is_none());
    }
}
