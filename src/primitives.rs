use std::{
    fs::{File, read, read_dir},
    io::{self, Cursor, Write, stdout},
    path::{Path, PathBuf},
    process::exit,
    sync::{Arc, atomic::Ordering},
    thread::{sleep, spawn},
    time::{Duration, Instant},
};

use crossterm::terminal;
use rodio::{Decoder, OutputStreamBuilder, Sink, Source};

use crate::{
    Res, STOP, TEMP_DIR,
    backup_counter::{SYNC_COUNTER, outside_counter},
    ffmpeg::{extract_audio, split_video_frames},
    messages::FRAMETIME_ZERO,
};

pub struct Bapple {
    frames: Box<[PathBuf]>, // Paths to the extracted frames, read on demand
    audio: Arc<[u8]>,       // May be empty
    has_audio: bool,
    frametime: Duration,
    counter: usize,
    length: usize,
}

impl Drop for Bapple {
    fn drop(&mut self) {
        let _ = show_cursor(&mut stdout().lock());
    }
}

impl Bapple {
    pub fn new(video_file: &str) -> Res<Self> {
        let tmp_dir = &*TEMP_DIR;

        println!("Extracting frames and audio...");
        split_video_frames(video_file)?;

        let has_audio = extract_audio(video_file).is_ok();

        let mut entries: Vec<_> = read_dir(tmp_dir)?
            .filter_map(Result::ok)
            .filter(|e| e.file_name() != "audio.mp3")
            .collect();

        let parse_stem = |p: PathBuf| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u64>().ok())
        };

        if entries.iter().any(|e| parse_stem(e.path()).is_none()) {
            return Err(
                "Somehow, ffmpeg returned a file name that's not a number."
                    .into(),
            );
        }

        entries.sort_by_key(|e| parse_stem(e.path()).unwrap());

        let frames: Box<[PathBuf]> =
            entries.into_iter().map(|e| e.path()).collect();

        let audio = if has_audio {
            read(tmp_dir.join("audio.mp3"))?
        } else {
            Vec::new()
        };

        let frametime = Duration::from_secs_f64(1.0 / 24.0);

        Ok(Self {
            length: frames.len(),
            frames,
            audio: audio.into(),
            has_audio,
            frametime,
            counter: 0,
        })
    }

    pub fn play(&mut self) -> Res<()> {
        if self.frametime.is_zero() {
            eprintln!("{FRAMETIME_ZERO}");
            exit(1);
        }

        let first_frame =
            self.frames.first().ok_or("ffmpeg produced no frames")?;
        let source_size = jpeg_dimensions(first_frame)?;
        let mut term_size = terminal::size()?;

        #[cfg(target_os = "linux")]
        if self.has_audio {
            Self::check_alsa_config();
        }

        let mut sink = None;
        let mut total = None;

        // Don't drop prematurely, or else the audio won't play.
        let output_stream = OutputStreamBuilder::open_default_stream()?;

        if self.has_audio {
            let decoder = Decoder::new_mp3(Cursor::new(self.audio.clone()))?;
            let inner_total = decoder.total_duration().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Unable to determine audio duration",
                )
            })?;
            total = Some(inner_total);
            let source = decoder.track_position();

            let inner_sink = Sink::connect_new(output_stream.mixer());
            inner_sink.append(source);
            inner_sink.play();
            sink = Some(inner_sink);
        } else {
            let frametime = self.frametime;
            let length = self.length;
            spawn(move || outside_counter(frametime, length));
        }

        let mut lock = stdout().lock();

        #[cfg(windows)]
        enable_virtual_terminal_processing();

        clear(&mut lock)?;
        hide_cursor(&mut lock)?;

        while self.counter < self.length {
            if STOP.load(Ordering::Relaxed) {
                break;
            }

            let task_time = Instant::now();

            // Checked every frame, so the window can be resized while playing.
            let current_size = terminal::size()?;
            if current_size != term_size {
                term_size = current_size;
                clear(&mut lock)?; // Drop leftovers from the old size
            }
            let dimensions =
                fit_to_terminal(source_size, term_size.0, term_size.1);

            let ascii_frame =
                match make_ascii(&self.frames[self.counter], dimensions) {
                    Ok(frame) => frame,
                    // Ctrl-C deletes the temp dir while we may still be reading
                    Err(_) if STOP.load(Ordering::Relaxed) => break,
                    Err(e) => return Err(e),
                };

            return_home(&mut lock)?;
            lock.write_all(ascii_frame.as_bytes())?;
            lock.flush()?;

            if !self.counter.is_multiple_of(15) {
                self.counter += 1;
            } else if self.has_audio {
                // Same condition, safe unwrap.
                self.counter =
                    self.get_pos(sink.as_ref().unwrap(), total.unwrap());
            } else {
                self.backup_resync();
            }

            if let Some(remaining) =
                self.frametime.checked_sub(task_time.elapsed())
            {
                sleep(remaining);
            }
        }

        show_cursor(&mut lock)?;
        self.counter = 0;
        SYNC_COUNTER.store(0, Ordering::Relaxed);
        Ok(())
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    fn get_pos(&self, sink: &Sink, total: Duration) -> usize {
        (sink.get_pos().div_duration_f64(total) * self.length as f64).round()
            as usize
    }

    pub fn backup_resync(&mut self) {
        self.counter = SYNC_COUNTER.load(Ordering::Relaxed);
    }

    #[cfg(target_os = "linux")]
    fn check_alsa_config() {
        use crate::messages::ALSA_WARNING;
        use std::{path::Path, thread::sleep, time::Duration};

        if !Path::new("/etc/alsa/conf.d").exists() {
            eprintln!("{ALSA_WARNING}");
            sleep(Duration::from_secs(5));
        }
    }
}

use libasciic::{AsciiBuilder, FilterType, Style};
fn make_ascii(frame: &Path, (width, height): (u32, u32)) -> Res<String> {
    Ok(AsciiBuilder::new(File::open(frame)?)
        .dimensions(width, height)
        .filter_type(FilterType::Lanczos3)
        .colorize(true)
        .style(Style::Mixed)
        .threshold(5)
        .charset(".:-+=#@")
        .background_brightness(0.6)
        .make_ascii()?)
}

// libasciic resizes to the exact size we give it, so the aspect ratio
// (and the 1:2 shape of terminal cells) is on us.
// Fits into cols x rows, never upscales.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn fit_to_terminal(
    (src_w, src_h): (u32, u32),
    cols: u16,
    rows: u16,
) -> (u32, u32) {
    let (w, h) = (f64::from(src_w), f64::from(src_h));
    let scale = (f64::from(cols) / w).min(f64::from(rows) * 2.0 / h).min(1.0);

    (((w * scale) as u32).max(1), ((h * scale / 2.0) as u32).max(1))
}

// Walks the JPEG markers until the start-of-frame one, which holds the size.
fn jpeg_dimensions(path: &Path) -> Res<(u32, u32)> {
    let data = read(path)?;

    if !data.starts_with(&[0xFF, 0xD8]) {
        return Err("Extracted frame is not a JPEG".into());
    }

    let mut i = 2;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }

        match data[i + 1] {
            // Padding, and markers that have no length
            0x00 | 0x01 | 0xD0..=0xD8 => i += 2,
            0xFF => i += 1,
            // End of image / start of scan, no size found before them
            0xD9 | 0xDA => break,
            // SOF0-SOF15, minus DHT, JPG and DAC
            m @ 0xC0..=0xCF if !matches!(m, 0xC4 | 0xC8 | 0xCC) => {
                if i + 9 > data.len() {
                    break;
                }
                let height = u16::from_be_bytes([data[i + 5], data[i + 6]]);
                let width = u16::from_be_bytes([data[i + 7], data[i + 8]]);
                return Ok((width.into(), height.into()));
            }
            _ => {
                let len = u16::from_be_bytes([data[i + 2], data[i + 3]]);
                i += 2 + usize::from(len);
            }
        }
    }

    Err("Could not find the size of the extracted frame".into())
}

#[cfg(windows)]
fn enable_virtual_terminal_processing() {
    use winapi::um::consoleapi::GetConsoleMode;
    use winapi::um::consoleapi::SetConsoleMode;
    use winapi::um::handleapi::INVALID_HANDLE_VALUE;
    use winapi::um::processenv::GetStdHandle;
    use winapi::um::winbase::STD_OUTPUT_HANDLE;
    use winapi::um::wincon::ENABLE_VIRTUAL_TERMINAL_PROCESSING;

    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if handle != INVALID_HANDLE_VALUE {
            let mut mode = 0;
            if GetConsoleMode(handle, &raw mut mode) != 0 {
                if SetConsoleMode(
                    handle,
                    mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
                ) == 0
                {
                    eprintln!(
                        "Warning: Failed to enable virtual terminal processing"
                    );
                }
            } else {
                eprintln!("Warning: Failed to get console mode");
            }
        }
    }
}

macro_rules! write_fn {
    ($fn_name:ident, $val:expr) => {
        #[inline]
        fn $fn_name<W: std::io::Write>(w: &mut W) -> std::io::Result<()> {
            w.write_all($val)
        }
    };
}

write_fn!(clear, b"\r\x1b[2J\x1b[H");
write_fn!(show_cursor, b"\x1b[?25h");
write_fn!(hide_cursor, b"\x1b[?25l");
write_fn!(return_home, b"\x1b[H");
