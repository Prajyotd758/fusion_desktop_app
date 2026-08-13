use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const BAUD_RATE: u32 = 921_600;
const SAMPLE_RATE: u32 = 16_000;

struct RecordingHandle {
    stop_tx: Sender<()>,
    done_rx: mpsc::Receiver<Result<(), String>>,
}

pub struct AudioState {
    pub recording: AtomicBool,
    cpal_handle: Mutex<Option<RecordingHandle>>,
    /// Shared f32 mono 16kHz buffer, filled by whichever path is active.
    pub samples: Arc<Mutex<Vec<f32>>>,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            recording: AtomicBool::new(false),
            cpal_handle: Mutex::new(None),
            samples: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

pub fn is_recording(state: &AudioState) -> bool {
    state.recording.load(Ordering::SeqCst)
}

/// Call after STOP to grab and clear the buffer.
pub fn take_samples(state: &AudioState) -> Vec<f32> {
    std::mem::take(&mut *state.samples.lock().unwrap())
}

// ---------------------------------------------------------------------
// Mouse trigger path: ESP32 over serial (already 16kHz mono i16 PCM)
// ---------------------------------------------------------------------

pub fn start_serial_listener(app: AppHandle) {
    std::thread::spawn(move || loop {
        let Some(port_name) = crate::find_serial::find_airgrip_port() else {
            std::thread::sleep(std::time::Duration::from_secs(2));
            continue;
        };

        match serialport::new(&port_name, BAUD_RATE)
            .timeout(std::time::Duration::from_millis(500))
            .open()
        {
            Ok(mut port) => {
                let _ = port.write_data_terminal_ready(true);
                let _ = port.write_request_to_send(true);
                std::thread::sleep(std::time::Duration::from_millis(500));
                if let Err(e) = run_listener(&app, port) {
                    eprintln!("[serial] listener error: {}", e);
                }
            }
            Err(e) => {
                eprintln!("[serial] failed to open {}: {} (retrying...)", port_name, e);
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    });
}

fn run_listener(app: &AppHandle, port: Box<dyn serialport::SerialPort>) -> Result<()> {
    let mut port = port;
    let mut line_buf: Vec<u8> = Vec::new();

    let mut byte = [0u8; 1];
    let mut state = 0u8;
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
                                let state_handle = app.state::<AudioState>();
                                if state_handle.recording.swap(true, Ordering::SeqCst) {
                                    eprintln!("[serial] START ignored, already recording");
                                    continue;
                                }
                                state_handle.samples.lock().unwrap().clear();
                                total_samples = 0;
                            } else if line == "STOP" {
                                let state_handle = app.state::<AudioState>();
                                state_handle.recording.store(false, Ordering::SeqCst);

                                println!("Total samples written = {}", total_samples);
                                println!(
                                    "Expected duration = {:.2} sec",
                                    total_samples as f32 / SAMPLE_RATE as f32
                                );

                                if total_samples == 0 {
                                    eprintln!("[serial] zero-sample session, skipping transcribe");
                                    continue;
                                }

                                let samples = take_samples(&state_handle);
                                let app_clone = app.clone();
                                tauri::async_runtime::spawn(async move {
                                    let (llm_choice, keys, language) = {
                                        let settings_state =
                                            app_clone.state::<crate::settings::SettingsState>();
                                        let settings = settings_state.0.lock().unwrap();
                                        (
                                            settings.llm_choice,
                                            settings.api_keys.clone(),
                                            settings.current_language().to_string(),
                                        )
                                    };

                                    match crate::commands::run_transcribe_only(
                                        &app_clone, llm_choice, &keys, &language, samples,
                                    )
                                    .await
                                    {
                                        Ok(result) => {
                                            let _ = app_clone.emit("command-result", result);
                                        }
                                        Err(e) => eprintln!("[serial] transcribe failed: {e}"),
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
                            let state_handle = app.state::<AudioState>();
                            let mut buf = state_handle.samples.lock().unwrap();
                            for chunk in payload.chunks_exact(2) {
                                let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                                buf.push(sample as f32 / i16::MAX as f32);
                                total_samples += 1;
                            }
                            state = 0;
                        }
                    }
                    _ => state = 0,
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => return Err(anyhow!("Serial read error: {}", e)),
        }
    }
}

// ---------------------------------------------------------------------
// Keyboard trigger path: cpal-based recording
// ---------------------------------------------------------------------

pub fn start_recording(state: &AudioState) -> Result<()> {
    if state.recording.swap(true, Ordering::SeqCst) {
        return Err(anyhow!("Recording already in progress"));
    }

    let mut guard = state.cpal_handle.lock().unwrap();
    if guard.is_some() {
        state.recording.store(false, Ordering::SeqCst);
        return Err(anyhow!("Recording already in progress"));
    }

    state.samples.lock().unwrap().clear();
    let samples_buf = state.samples.clone();

    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel::<Result<(), String>>();

    thread::spawn(move || {
        let result = (|| -> Result<()> {
            let host = cpal::default_host();
            let device = host
                .default_input_device()
                .ok_or_else(|| anyhow!("No input device found"))?;
            let config = device.default_input_config()?;
            let channels = config.channels() as usize;
            let src_rate = config.sample_rate().0;

            if src_rate != SAMPLE_RATE {
                eprintln!(
                    "[cpal] WARNING: mic rate {}Hz != {}Hz, whisper needs resampling (TODO: rubato)",
                    src_rate, SAMPLE_RATE
                );
            }

            let err_fn = |err| eprintln!("Stream error: {}", err);

            let push = move |mono: Vec<f32>, buf: &Arc<Mutex<Vec<f32>>>| {
                buf.lock().unwrap().extend(mono);
            };

            let stream = match config.sample_format() {
                SampleFormat::F32 => {
                    let buf = samples_buf.clone();
                    device.build_input_stream(
                        &config.clone().into(),
                        move |data: &[f32], _| {
                            let mono: Vec<f32> = data
                                .chunks_exact(channels)
                                .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                                .collect();
                            push(mono, &buf);
                        },
                        err_fn,
                        None,
                    )?
                }
                SampleFormat::I16 => {
                    let buf = samples_buf.clone();
                    device.build_input_stream(
                        &config.clone().into(),
                        move |data: &[i16], _| {
                            let mono: Vec<f32> = data
                                .chunks_exact(channels)
                                .map(|frame| {
                                    frame.iter().map(|&s| s as f32).sum::<f32>()
                                        / channels as f32
                                        / i16::MAX as f32
                                })
                                .collect();
                            push(mono, &buf);
                        },
                        err_fn,
                        None,
                    )?
                }
                SampleFormat::U16 => {
                    let buf = samples_buf.clone();
                    device.build_input_stream(
                        &config.clone().into(),
                        move |data: &[u16], _| {
                            let mono: Vec<f32> = data
                                .chunks_exact(channels)
                                .map(|frame| {
                                    frame
                                        .iter()
                                        .map(|&s| (s as i32 - 32768) as f32)
                                        .sum::<f32>()
                                        / channels as f32
                                        / i16::MAX as f32
                                })
                                .collect();
                            push(mono, &buf);
                        },
                        err_fn,
                        None,
                    )?
                }
                _ => return Err(anyhow!("Unsupported sample format")),
            };

            stream.play()?;
            println!("Recording started...");
            let _ = stop_rx.recv();
            drop(stream);
            println!("Recording stopped, samples buffered in memory");
            Ok(())
        })();

        let _ = done_tx.send(result.map_err(|e| e.to_string()));
    });

    *guard = Some(RecordingHandle { stop_tx, done_rx });
    Ok(())
}

pub fn stop_recording(state: &AudioState) -> Result<()> {
    let handle = {
        let mut guard = state.cpal_handle.lock().unwrap();
        guard
            .take()
            .ok_or_else(|| anyhow!("No recording in progress"))?
    };

    handle.stop_tx.send(()).ok();

    let result = match handle.done_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(anyhow!(e)),
        Err(_) => Err(anyhow!("Timed out waiting for recording thread to finish")),
    };

    state.recording.store(false, Ordering::SeqCst);
    result
}
