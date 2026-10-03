#![warn(clippy::pedantic)]

use std::env::temp_dir;
use std::{env::args, sync::atomic::AtomicBool};
use std::{
    fmt::Debug,
    fs::{create_dir_all, remove_dir_all},
    path::PathBuf,
    sync::{LazyLock, Once, atomic::Ordering::SeqCst},
    thread::sleep,
    time::Duration,
};

use crate::{ffmpeg::FFMPEG_RUNNING, primitives::Bapple};

type Error = Box<dyn std::error::Error>;
type Res<T> = std::result::Result<T, Error>;

mod backup_counter;
mod colours;
mod ffmpeg;
mod installer;
mod messages;
mod primitives;

static STOP: AtomicBool = AtomicBool::new(false);

static TEMP_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    let dir = temp_dir().join(format!(".bjit-tmp-{}", std::process::id()));

    if dir.exists() {
        remove_dir_all(&dir).unwrap();
    }

    create_dir_all(&dir).unwrap();

    dir
});

fn main() {
    err_exit(run());
}

fn run() -> Res<()> {
    let once = Once::new();
    ctrlc::set_handler(move || once.call_once(ctrl_c))?;

    wait_for_resize();

    let video_file = args().nth(1).ok_or("Usage: bplay-jit <video>")?;

    let mut bapple = Bapple::new(&video_file)?;
    cleanup();

    bapple.play()?;
    Ok(())
}

fn wait_for_resize() {
    println!(
        "Resize this window however you want, \
        just be mindful that cmd.exe is a bottleneck."
    );
    println!("If you're using it, try not to maximise it :)");
    println!("-- Press Enter To Continue --");

    let _ = std::io::stdin().read_line(&mut String::new());
}

fn err_exit<T>(result: Result<T, impl Debug>) -> T {
    match result {
        Err(e) => {
            eprintln!("{e:?}");
            halt();
        }
        Ok(ok) => ok,
    }
}

fn halt() -> ! {
    println!("\nPress Ctrl-C to exit");
    loop {
        sleep(Duration::MAX);
    }
}

fn cleanup() {
    let tmp_dir = TEMP_DIR.clone();
    if !tmp_dir.exists() {
        return;
    }

    let timeout = Duration::from_secs(30);
    let start = std::time::Instant::now();

    while FFMPEG_RUNNING.load(SeqCst) {
        if start.elapsed() > timeout {
            eprintln!(
                "Warning: ffmpeg did not finish within timeout, proceeding with cleanup"
            );
            break;
        }
        sleep(Duration::from_millis(100));
    }

    println!("Cleaning up...");
    if let Err(e) = remove_dir_all(&tmp_dir) {
        eprintln!("{e}");
        eprintln!(
            "Cleanup function failed while trying to delete {}",
            tmp_dir.display()
        );
    }

    println!("Done!");
}

fn ctrl_c() {
    STOP.store(true, std::sync::atomic::Ordering::Relaxed);
    cleanup();

    sleep(Duration::from_secs(1));

    std::process::exit(0);
}
