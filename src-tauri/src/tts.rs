use anyhow::{bail, Result};
use rodio::{Decoder, OutputStream, Sink};
use std::{
    fs::File,
    io::{BufReader, Write},
    path::PathBuf,
    process::{Command, Stdio},
};

pub fn speak(text: &str) -> Result<()> {
    // Assumes the app is started from src-tauri during development.
    // Later, when bundling the app, we'll switch to Tauri's app.path() API.
    let root = std::env::current_dir()?;

    let piper = root.join("piper").join("piper.exe");
    let model = root.join("models").join("en_US-lessac-medium.onnx");

    // Store generated audio in the temp directory
    let output = std::env::temp_dir().join("fusion_response.wav");

    println!("Piper: {:?}", piper);
    println!("Model: {:?}", model);
    println!("Output: {:?}", output);

    // Launch Piper
    let mut child = Command::new(&piper)
        .arg("--model")
        .arg(&model)
        .arg("--output_file")
        .arg(&output)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Send the text to Piper
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(text.as_bytes())?;
    }

    // Wait for Piper to finish
    let output_result = child.wait_with_output()?;

    if !output_result.status.success() {
        println!(
            "Piper stderr:\n{}",
            String::from_utf8_lossy(&output_result.stderr)
        );

        println!(
            "Piper stdout:\n{}",
            String::from_utf8_lossy(&output_result.stdout)
        );

        bail!("Piper failed to generate speech.");
    }
    // Play the generated WAV
    play_wav(output)
}

fn play_wav(path: PathBuf) -> Result<()> {
    let (_stream, stream_handle) = OutputStream::try_default()?;

    let sink = Sink::try_new(&stream_handle)?;

    let file = BufReader::new(File::open(path)?);

    let source = Decoder::new(file)?;

    sink.append(source);

    // Wait until playback finishes
    sink.sleep_until_end();

    Ok(())
}
