// Suppresses warnings on other systems
#![cfg_attr(
    not(any(target_os = "windows", target_os = "linux")),
    allow(unreachable_code, unused_variables)
)]

use std::{
    fs::{self, File, create_dir_all},
    path::{Path, PathBuf},
};

use crate::{
    Res,
    colours::{BCYAN, BDIM, GREEN, RED, RESET, YELLOW},
};

use indicatif::{ProgressBar, ProgressStyle};
use which::which;

#[cfg(target_os = "linux")]
const FFMPEG_URL: &str = "https://github.com/S0raWasTaken/bapple_mirror/releases/download/latest/ffmpeg";

#[cfg(target_os = "windows")]
const FFMPEG_URL: &str = "https://github.com/S0raWasTaken/bapple_mirror/releases/download/latest/ffmpeg.exe";

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
const FFMPEG_URL: &str = "";

pub struct Dependencies {
    pub ffmpeg: PathBuf,
}

impl Dependencies {
    pub fn setup() -> Res<Self> {
        println!(
            "{BDIM}[1/5]{RESET}  {BCYAN}Resolving ffmpeg...{RESET}"
        );

        Ok(Self { ffmpeg: setup_ffmpeg()? })
    }
}

fn setup_ffmpeg() -> Res<PathBuf> {
    // Prefer the system install; only download if it's missing.
    if let Some(system_ffmpeg) = find_system_binary("ffmpeg") {
        return Ok(system_ffmpeg);
    }

    let data_dir = local_data_dir()?;
    let ffmpeg_output =
        data_dir.join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));

    // Reuse a previously downloaded copy if there is one.
    if !ffmpeg_output.exists() {
        create_dir_all(&data_dir)?;
        download_and_setup_binary(FFMPEG_URL, &ffmpeg_output)?;
    }

    Ok(ffmpeg_output)
}

const TEMPLATE: &str = "{spinner:.green} {msg} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})";
fn download_and_setup_binary(url: &str, output: &Path) -> Res<()> {
    let response = reqwest::blocking::get(url)?.error_for_status()?;
    let total_size = response.content_length().unwrap_or(0);

    let pb = ProgressBar::new(total_size);
    pb.set_style(ProgressStyle::with_template(TEMPLATE)?.progress_chars("██░"));
    pb.set_message(format!(
        "     {BCYAN}Downloading {RESET}{YELLOW}{} {BCYAN}from {RESET}{YELLOW}{}{RESET}\n      ",
        output.file_stem().unwrap().display(),
        url
    ));

    let temp_output = output.with_extension("tmp");
    let mut file = File::create(&temp_output)?;
    match std::io::copy(&mut pb.wrap_read(response), &mut file) {
        Ok(_) => {
            drop(file);
            if let Err(e) = fs::rename(&temp_output, output) {
                let _ = fs::remove_file(&temp_output);
                return Err(e.into());
            }
        }
        Err(e) => {
            drop(file);
            let _ = fs::remove_file(&temp_output);
            return Err(e.into());
        }
    }

    pb.finish_and_clear();
    println!(
        "       {BCYAN}Success! {RESET}{YELLOW}{}{RESET}",
        output.display()
    );

    #[cfg(unix)]
    fix_perms(output)?;

    Ok(())
}

#[inline]
fn local_data_dir() -> Res<PathBuf> {
    Ok(user_dirs::data_dir()?.join("asciic-bin"))
}

#[cfg(unix)]
fn fix_perms(file: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = fs::metadata(file)?.permissions();
    perms.set_mode(perms.mode() | 0o111);
    fs::set_permissions(file, perms)?;
    Ok(())
}

#[inline]
fn find_system_binary(name: &str) -> Option<PathBuf> {
    if let Ok(path) = which(name) {
        println!(
            "       {GREEN}Using system {name} binary at {}{RESET}",
            path.display()
        );
        Some(path)
    } else {
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            eprintln!(
                "{RED}{name} not found in PATH.\n\
                Automatic dependency management is not supported for this OS.{RESET}"
            );
            std::process::exit(1);
        }
        eprintln!(
            "{RED}{name} not found in PATH; falling back to bundled download.{RESET}"
        );
        None
    }
}
