use std::{
    path::PathBuf,
    process::Command,
    sync::{
        LazyLock,
        atomic::{AtomicBool, Ordering::SeqCst},
    },
};

use crate::{Res, TEMP_DIR, err_exit, installer::Dependencies};

static FFMPEG_PATH: LazyLock<PathBuf> = LazyLock::new(|| {
    let deps = err_exit(Dependencies::setup());
    deps.ffmpeg
});

pub fn split_video_frames(video_path: &str) -> Res<()> {
    // Frames are extracted at full resolution, libasciic does the resizing.
    ffmpeg(&[
        "-hwaccel",
        "auto", // Falls back to software decoding if unavailable
        "-i",
        video_path,
        "-vf",
        "fps=24",
        "-q:v",
        "2", // JPEG quality, 2 is near-lossless and still very fast to encode
        &format!("{}/%03d.jpg", TEMP_DIR.to_string_lossy()),
    ])
}

pub fn extract_audio(video_path: &str) -> Res<()> {
    ffmpeg(&[
        "-i",
        video_path,
        "-vn",
        "-codec:a",
        "libmp3lame",
        &format!("{}/audio.mp3", TEMP_DIR.to_string_lossy()),
    ])
}

pub static FFMPEG_RUNNING: AtomicBool = AtomicBool::new(false);

const FFMPEG_FLAGS: [&str; 4] = ["-loglevel", "error", "-stats", "-nostdin"];
pub fn ffmpeg(args: &[&str]) -> Res<()> {
    FFMPEG_RUNNING.store(true, SeqCst);

    let status = Command::new(&*FFMPEG_PATH)
        .args(FFMPEG_FLAGS) // Default args
        .args(args)
        .status();

    FFMPEG_RUNNING.store(false, SeqCst);

    let status = status?;

    if !status.success() {
        return Err(format!(
            "FFMPEG failed with status: {{{}}}",
            status
                .code()
                .map_or_else(|| "TERMINATED".into(), |s| s.to_string())
        )
        .into());
    }

    Ok(())
}
