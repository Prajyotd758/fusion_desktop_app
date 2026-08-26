use anyhow::{bail, Result};
use hound::WavReader;
use rodio::buffer::SamplesBuffer;
use std::num::NonZero;
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};
use tauri::{AppHandle, Manager};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub fn speak(app: &AppHandle, text: &str) -> Result<()> {
    let piper_raw = app
        .path()
        .resolve("piper/piper.exe", tauri::path::BaseDirectory::Resource)?;
    let model_raw = app.path().resolve(
        "models/en_US-lessac-medium.onnx",
        tauri::path::BaseDirectory::Resource,
    )?;

    let piper = dunce::canonicalize(&piper_raw).unwrap_or(piper_raw);
    let model = dunce::canonicalize(&model_raw).unwrap_or(model_raw);

    let output = std::env::temp_dir().join("fusion_response.wav");

    let mut cmd = Command::new(&piper);
    cmd.arg("--model")
        .arg(&model)
        .arg("--output_file")
        .arg(&output)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd.spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }

    let output_result = child.wait_with_output()?;

    if !output_result.status.success() {
        bail!(
            "Piper failed to generate speech (exit {:?}): stdout={} stderr={}",
            output_result.status.code(),
            String::from_utf8_lossy(&output_result.stdout),
            String::from_utf8_lossy(&output_result.stderr)
        );
    }
    play_wav(output)
}

fn play_wav(path: PathBuf) -> Result<()> {
    let stream = rodio::DeviceSinkBuilder::open_default_sink()?;
    let player = rodio::Player::connect_new(stream.mixer());

    let mut reader = WavReader::open(&path)?;
    let spec = reader.spec();
    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / i16::MAX as f32)
        .collect();

    let channels = NonZero::new(spec.channels).expect("channels must be nonzero");
    let sample_rate = NonZero::new(spec.sample_rate).expect("sample rate must be nonzero");

    let source = SamplesBuffer::new(channels, sample_rate, samples);
    player.append(source);
    player.sleep_until_end();

    Ok(())
}
