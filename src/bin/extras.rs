#![allow(dead_code)]

use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};

#[path = "../config.rs"]
mod config;
#[path = "../notification.rs"]
mod notification;
#[path = "../utils.rs"]
mod utils;

use config::{config, Config, EXTRAS_PORT};
use notification::display_notification;
use utils::{capture, debug_log, get_hwnd, LogLevel};

const EXTRAS_TIMEOUT: u64 = 45;

fn main() {
    debug_log(LogLevel::INFO, "Starting Extras", None);

    if get_hwnd().is_none() {
        debug_log(LogLevel::ERROR, "Target window not found, exiting", None);
        return;
    }

    let listener = match TcpListener::bind(format!("127.0.0.1:{}", EXTRAS_PORT)) {
        Ok(l) => l,
        Err(e) => {
            debug_log(LogLevel::ERROR, &format!("Failed to bind TCP listener: {}", e), None);
            return;
        }
    };
    listener.set_nonblocking(true).unwrap();

    type RawFrame = (Vec<u8>, u32, u32, u64); // (pixels, width, height, sequence)
    let latest_frame: Arc<Mutex<Option<RawFrame>>> = Arc::new(Mutex::new(None));
    let last_cmd: Arc<Mutex<Instant>> = Arc::new(Mutex::new(Instant::now()));
    let frame_sequence: Arc<Mutex<u64>> = Arc::new(Mutex::new(0));
    let last_sent_sequence: Arc<Mutex<u64>> = Arc::new(Mutex::new(0));
    let should_exit: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));

    // Capture
    {
        let latest_frame = Arc::clone(&latest_frame);
        let last_cmd = Arc::clone(&last_cmd);
        let frame_sequence = Arc::clone(&frame_sequence);
        let should_exit = Arc::clone(&should_exit);

        thread::spawn(move || {
            let mut frame_count = 0;
            let mut last_log_time = Instant::now();
            let mut last_frame_time = Instant::now();

            loop {
                if get_hwnd().is_none() {
                    debug_log(LogLevel::INFO, "Target window not founded, exiting", None);
                    *should_exit.lock().unwrap() = true;
                    break;
                }

                if last_cmd.lock().unwrap().elapsed() > Duration::from_secs(EXTRAS_TIMEOUT) {
                    debug_log(LogLevel::INFO, "Command timeout exceeded, exiting", None);
                    *should_exit.lock().unwrap() = true;
                    break;
                }

                // FPS limit
                Config::reload();
                let max_fps = config().max_fps;
                let target_frame_duration = Duration::from_millis(1000 / max_fps as u64);
                let elapsed_since_last_frame = last_frame_time.elapsed();

                if elapsed_since_last_frame < target_frame_duration {
                    let sleep_duration = target_frame_duration - elapsed_since_last_frame;
                    thread::sleep(sleep_duration);
                }
                last_frame_time = Instant::now();

                let img = capture();
                let rgba = img.into_rgba8();
                let (w, h) = (rgba.width(), rgba.height());
                let raw = rgba.into_raw();

                let seq = {
                    let mut seq_guard = frame_sequence.lock().unwrap();
                    *seq_guard += 1;
                    *seq_guard
                };

                {
                    let mut guard = latest_frame.lock().unwrap();
                    *guard = Some((raw, w, h, seq));
                }

                frame_count += 1;
                let now = Instant::now();
                let elapsed = now.duration_since(last_log_time);

                if elapsed >= Duration::from_secs(1) {
                    if config().debug {
                        let fps = frame_count as f32 / elapsed.as_secs_f32();
                        let max_fps = config().max_fps;
                        debug_log(LogLevel::INFO, &format!("Capture FPS: {:.2}/{}", fps, max_fps), None);
                    }
                    frame_count = 0;
                    last_log_time = now;
                }
            }
        });
    }

    // Request
    loop {
        if *should_exit.lock().unwrap() {
            display_notification(LogLevel::INFO, "extras_stop", &[]);
            break;
        }

        match listener.accept() {
            Ok((mut stream, _addr)) => {
                *last_cmd.lock().unwrap() = Instant::now();
                Config::reload();

                // Read command
                let mut cmd_buf = [0u8; 3];
                if stream.read_exact(&mut cmd_buf).is_err() {
                    continue;
                }

                if &cmd_buf == b"INV" {
                    // Update last_sent_sequence to current sequence
                    let latest_seq = latest_frame.lock().unwrap().as_ref().map(|f| f.3).unwrap_or(0);
                    *last_sent_sequence.lock().unwrap() = latest_seq;
                    let _ = stream.shutdown(Shutdown::Write);
                    continue;
                }

                // GET
                let frame_opt = loop {
                    // Check exit flag during frame waiting
                    if *should_exit.lock().unwrap() {
                        break None;
                    }

                    let current_frame = {
                        let guard = latest_frame.lock().unwrap();
                        guard.clone()
                    };

                    if let Some((_, _, _, seq)) = &current_frame {
                        let last_sent = *last_sent_sequence.lock().unwrap();
                        if *seq > last_sent {
                            break current_frame;
                        }
                    }

                    thread::sleep(Duration::from_millis(5));
                };

                if let Some((raw, w, h, seq)) = frame_opt {
                    if let Err(e) = send_frame(&mut stream, &raw, w, h) {
                        debug_log(LogLevel::ERROR, &format!("Failed to send frame: {}", e), None);
                    } else {
                        *last_sent_sequence.lock().unwrap() = seq;
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break,
        }
    }
}

fn send_frame(stream: &mut TcpStream, raw: &[u8], w: u32, h: u32) -> std::io::Result<()> {
    let mut png_buf = Vec::<u8>::new();
    PngEncoder::new(&mut png_buf).write_image(raw, w, h, ColorType::Rgba8.into()).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    stream.write_all(&png_buf)?;
    stream.shutdown(Shutdown::Write)?;
    Ok(())
}
