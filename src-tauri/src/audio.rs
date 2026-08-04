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
    /// Shared flag: true whenever EITHER the mouse (serial) or keyboard (cpal) path is recording.
    pub recording: AtomicBool,
    /// Only used by the keyboard/cpal path.
    cpal_handle: Mutex<Option<RecordingHandle>>,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            recording: AtomicBool::new(false),
            cpal_handle: Mutex::new(None),
        }
    }
}

pub fn is_recording(state: &AudioState) -> bool {
    state.recording.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------------
// Mouse trigger path: ESP32 over serial (START/STOP + binary PCM frames)
// ---------------------------------------------------------------------

pub fn start_serial_listener(app: AppHandle) {
    std::thread::spawn(move || loop {
        let Some(port_name) = crate::find_serial::find_airgrip_port() else {
            // eprintln!("[serial] AirGrip not found, retrying...");
            std::thread::sleep(std::time::Duration::from_secs(2));
            continue;
        };

        match serialport::new(&port_name, BAUD_RATE)
            .timeout(std::time::Duration::from_millis(500))
            .open()
        {
            Ok(mut port) => {
                eprintln!("[serial] connected to {}", port_name);
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

    let mut writer: Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>> = None;
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

                                // Guard: don't start if the keyboard/cpal path already owns recording.
                                if state_handle.recording.swap(true, Ordering::SeqCst) {
                                    eprintln!("[serial] START ignored, already recording (possibly via shortcut)");
                                    continue;
                                }

                                let spec = hound::WavSpec {
                                    channels: 1,
                                    sample_rate: SAMPLE_RATE,
                                    bits_per_sample: 16,
                                    sample_format: hound::SampleFormat::Int,
                                };
                                match hound::WavWriter::create("../../data/audio/latest.wav", spec)
                                {
                                    Ok(w) => writer = Some(w),
                                    Err(e) => {
                                        eprintln!("[serial] failed to create WAV writer: {e}");
                                        state_handle.recording.store(false, Ordering::SeqCst);
                                    }
                                }
                                total_samples = 0;
                            } else if line == "STOP" {
                                let state_handle = app.state::<AudioState>();
                                state_handle.recording.store(false, Ordering::SeqCst);

                                if let Some(w) = writer.take() {
                                    println!("Total samples written = {}", total_samples);
                                    println!(
                                        "Expected duration = {:.2} sec",
                                        total_samples as f32 / SAMPLE_RATE as f32
                                    );
                                    w.finalize().ok();
                                }

                                if total_samples == 0 {
                                    eprintln!("[serial] zero-sample session, skipping transcribe");
                                    continue;
                                }

                                let app_clone = app.clone();
                                tauri::async_runtime::spawn(async move {
                                    // Pull current llm_choice + api_keys from persisted settings
                                    // right before running the transcription pipeline.
                                    let (llm_choice, keys) = {
                                        let settings_state =
                                            app_clone.state::<crate::settings::SettingsState>();
                                        let settings = settings_state.0.lock().unwrap();
                                        (settings.llm_choice, settings.api_keys.clone())
                                    };

                                    match crate::commands::run_transcribe_only(llm_choice, &keys)
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
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
            Err(e) => return Err(anyhow!("Serial read error: {}", e)),
        }
    }
}
// ---------------------------------------------------------------------
// Keyboard trigger path: cpal-based recording, controlled via shortcut
// ---------------------------------------------------------------------

pub fn start_recording(state: &AudioState) -> Result<()> {
    // Guard: don't start if serial/mouse path already owns recording.
    if state.recording.swap(true, Ordering::SeqCst) {
        return Err(anyhow!("Recording already in progress"));
    }

    let mut guard = state.cpal_handle.lock().unwrap();
    if guard.is_some() {
        state.recording.store(false, Ordering::SeqCst);
        return Err(anyhow!("Recording already in progress"));
    }

    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel::<Result<(), String>>();

    thread::spawn(move || {
        let result = (|| -> Result<()> {
            let host = cpal::default_host();
            let device = host
                .default_input_device()
                .ok_or_else(|| anyhow!("No input device found"))?;
            let config = device.default_input_config()?;

            let spec = hound::WavSpec {
                channels: config.channels(),
                sample_rate: config.sample_rate().0,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };

            let writer = Arc::new(Mutex::new(Some(hound::WavWriter::create(
                "../../data/audio/latest.wav",
                spec,
            )?)));
            let writer_clone = writer.clone();
            let err_fn = |err| eprintln!("Stream error: {}", err);

            let stream = match config.sample_format() {
                SampleFormat::F32 => device.build_input_stream(
                    &config.clone().into(),
                    move |data: &[f32], _| {
                        let mut w = writer_clone.lock().unwrap();
                        if let Some(w) = w.as_mut() {
                            for &sample in data {
                                let s = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                                w.write_sample(s).ok();
                            }
                        }
                    },
                    err_fn,
                    None,
                )?,
                SampleFormat::I16 => device.build_input_stream(
                    &config.clone().into(),
                    move |data: &[i16], _| {
                        let mut w = writer_clone.lock().unwrap();
                        if let Some(w) = w.as_mut() {
                            for &sample in data {
                                w.write_sample(sample).ok();
                            }
                        }
                    },
                    err_fn,
                    None,
                )?,
                SampleFormat::U16 => device.build_input_stream(
                    &config.clone().into(),
                    move |data: &[u16], _| {
                        let mut w = writer_clone.lock().unwrap();
                        if let Some(w) = w.as_mut() {
                            for &sample in data {
                                let s = (sample as i32 - 32768) as i16;
                                w.write_sample(s).ok();
                            }
                        }
                    },
                    err_fn,
                    None,
                )?,
                _ => return Err(anyhow!("Unsupported sample format")),
            };

            stream.play()?;
            println!("Recording started...");

            let _ = stop_rx.recv();
            drop(stream);

            let mut w = writer.lock().unwrap();
            if let Some(w) = w.take() {
                w.finalize()?;
            }

            println!("Recording stopped, saved to latest.wav");
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
