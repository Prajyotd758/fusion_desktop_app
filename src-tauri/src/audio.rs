use anyhow::{anyhow, Result};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

const SERIAL_PORT: &str = "COM12"; // change to your ESP32 port
const BAUD_RATE: u32 = 921_600;
const SAMPLE_RATE: u32 = 16_000;

pub struct AudioState {
    pub recording: AtomicBool,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            recording: AtomicBool::new(false),
        }
    }
}

pub fn is_recording(state: &AudioState) -> bool {
    state.recording.load(Ordering::SeqCst)
}

/// Spawns a persistent background thread that listens to the ESP32 over serial.
/// Reacts to "START"/"STOP" text markers to control recording,
/// and streams binary PCM frames (0xAB 0xCD [len_lo][len_hi][data]) into a WAV file.
pub fn start_serial_listener(app: AppHandle) {
    std::thread::spawn(move || loop {
        match serialport::new(SERIAL_PORT, BAUD_RATE)
            .timeout(std::time::Duration::from_millis(500))
            .open()
        {
            Ok(mut port) => {
                eprintln!("[serial] connected to {}", SERIAL_PORT);
                let _ = port.write_data_terminal_ready(true);
                let _ = port.write_request_to_send(true);
                std::thread::sleep(std::time::Duration::from_millis(500)); // let ESP32 settle
                if let Err(e) = run_listener(&app, port) {
                    eprintln!("[serial] listener error: {}", e);
                }
            }
            Err(e) => {
                eprintln!(
                    "[serial] failed to open {}: {} (retrying...)",
                    SERIAL_PORT, e
                );
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(2)); // retry if disconnected
    });
}

fn run_listener(app: &AppHandle, port: Box<dyn serialport::SerialPort>) -> Result<()> {
    let mut port = port;

    let mut writer: Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>> = None;
    let mut line_buf: Vec<u8> = Vec::new();

    let mut byte = [0u8; 1];
    let mut state = 0u8; // 0=idle/text, 1=got 0xAB, 2=reading len, 3=reading payload
    let mut len_bytes = [0u8; 2];
    let mut len_idx = 0;
    let mut payload_len = 0usize;
    let mut payload: Vec<u8> = Vec::new();
    let mut total_samples = 0;

    loop {
        match port.read(&mut byte) {
            Ok(0) => continue,
            Ok(_) => {
                let b = byte[0];

                if state == 0 && b == 0xAB {
                    state = 1;
                    line_buf.clear();
                    continue;
                }

                match state {
                    0 => {
                        if b == b'\n' {
                            let line = String::from_utf8_lossy(&line_buf).trim().to_string();
                            line_buf.clear();

                            if line == "START" {
                                let state_handle = app.state::<Arc<AudioState>>();
                                state_handle.recording.store(true, Ordering::SeqCst);

                                let spec = hound::WavSpec {
                                    channels: 1,
                                    sample_rate: SAMPLE_RATE,
                                    bits_per_sample: 16,
                                    sample_format: hound::SampleFormat::Int,
                                };
                                writer =
                                    hound::WavWriter::create("../../data/audio/latest.wav", spec)
                                        .ok();
                            } else if line == "STOP" {
                                let state_handle = app.state::<Arc<AudioState>>();
                                state_handle.recording.store(false, Ordering::SeqCst);

                                if let Some(w) = writer.take() {
                                    println!("Total samples written = {}", total_samples);
                                    println!(
                                        "Expected duration = {:.2} sec",
                                        total_samples as f32 / SAMPLE_RATE as f32
                                    );
                                    w.finalize().ok();
                                }

                                let app_clone = app.clone();
                                tauri::async_runtime::spawn(async move {
                                    match crate::commands::run_transcribe_only().await {
                                        Ok(result) => {
                                            let _ = app_clone.emit("command-result", result);
                                        }
                                        Err(e) => eprintln!("[] failed: {e}"),
                                    }
                                });
                            }
                        } else {
                            line_buf.push(b);
                        }
                    }
                    1 => {
                        state = if b == 0xCD { 2 } else { 0 };
                        len_idx = 0;
                    }
                    2 => {
                        len_bytes[len_idx] = b;
                        len_idx += 1;
                        if len_idx == 2 {
                            payload_len = (len_bytes[0] as usize) | ((len_bytes[1] as usize) << 8);

                            payload.clear();
                            state = if payload_len == 0 { 0 } else { 3 };
                        }
                    }
                    3 => {
                        payload.push(b);
                        if payload.len() == payload_len {
                            if let Some(w) = writer.as_mut() {
                                for chunk in payload.chunks_exact(2) {
                                    let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                                    w.write_sample(sample).ok();
                                    total_samples += 1;
                                }
                            }
                            state = 0;
                        }
                    }
                    _ => state = 0,
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                continue;
            }
            Err(e) => return Err(anyhow!("Serial read error: {}", e)),
        }
    }
}
