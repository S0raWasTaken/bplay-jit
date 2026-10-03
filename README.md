# bplay-jit - Direct IO Version
A fork of [bapple_player](https://github.com/S0raWasTaken/bapple_player) that doesn't need pre-rendered .bapple files.
It takes any video and turns the frames into ASCII while playing them, and it stays in sync with the audio.

This branch includes the Direct IO version of the player. In this one, frames are stored in the disk throughout the duration of the
video, and they're resized during playback, making it possible to resize the video while playing.

The only con is that since we're not using ffmpeg to resize, frame splitting takes way longer.

### Installation
```sh
git clone https://github.com/S0raWasTaken/bapple_player --depth 1
cargo install --path bapple_player
```
If ffmpeg is in your PATH, it'll be used. If not, it gets downloaded automatically (Linux and Windows only).

This works on Windows, but cmd and PowerShell are not GPU-accelerated, so they're a huge bottleneck.
Use something like [kitty](https://github.com/kovidgoyal/kitty) or Windows Terminal instead, and if you really must use cmd, don't maximize it.

### Usage
```sh
bplay-jit video.mp4
```
It'll ask you to resize the window and press Enter. Do the resizing first, the video is scaled to whatever size the terminal has at that point.

You can also set it as the "Open With" program for your video files, as long as it opens in a terminal.

Ctrl-C to stop.