use std::{
    fs::{read, read_dir},
    io::{self, Cursor, Write, stdout},
    path::PathBuf,
    process::exit,
    sync::{Arc, atomic::Ordering},
    thread::{sleep, spawn},
    time::{Duration, Instant},
};

use rodio::{Decoder, OutputStreamBuilder, Sink, Source};

use crate::{
    Res, STOP, TEMP_DIR,
    backup_counter::{SYNC_COUNTER, outside_counter},
    ffmpeg::{extract_audio, split_video_frames},
    messages::FRAMETIME_ZERO,
};

pub struct Bapple {
    frames: Box<[Box<[u8]>]>,
    audio: Arc<[u8]>, // May be empty
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

        let frames: Box<[Box<[u8]>]> = entries
            .into_iter()
            .filter_map(|e| read(e.path()).ok())
            .map(Vec::into_boxed_slice)
            .collect();

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
            let ascii_frame = make_ascii(&self.frames[self.counter])?;

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

use libasciic::{AsciiBuilder, AsciiError, Style};
fn make_ascii(raw_frame: &[u8]) -> Result<String, AsciiError> {
    AsciiBuilder::new(Cursor::new(raw_frame))
        .colorize(true)
        .style(Style::Mixed)
        .threshold(5)
        .charset(".:-+=#@")
        .background_brightness(0.6)
        .make_ascii()
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
