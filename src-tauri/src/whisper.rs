use crate::system::helper_functions::groq_key;
use crate::whisper_engine::WhisperState;
use anyhow::Result;
use tauri::Manager;

pub async fn transcribe(
    app: &tauri::AppHandle,
    samples: Vec<f32>,
    language: &str,
) -> Result<String> {
    let language = language.to_string();
    let state = app.state::<WhisperState>();
    let engine_arc = state.0.clone();

    let text = tauri::async_runtime::spawn_blocking(move || {
        let engine = engine_arc.lock().unwrap();
        engine.transcribe(&samples, &language)
    })
    .await??;

    println!("text : {}", text);
    Ok(text)
}

pub async fn transcribe_groq(samples: Vec<f32>, language: &str) -> Result<String> {
    use std::io::Cursor;

    println!("language : {}", language.to_string());

    let mut wav = Cursor::new(Vec::new());

    {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };

        let mut writer = hound::WavWriter::new(&mut wav, spec)?;

        for sample in samples {
            let sample = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer.write_sample(sample)?;
        }

        writer.finalize()?;
    }

    let client = reqwest::Client::new();

    let part = reqwest::multipart::Part::bytes(wav.into_inner())
        .file_name("audio.wav")
        .mime_str("audio/wav")?;

    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", "whisper-large-v3-turbo")
        .text("language", language.to_string())
        .text("response_format", "json");

    let resp = client
        .post("https://api.groq.com/openai/v1/audio/transcriptions")
        .header("Authorization", format!("Bearer {}", groq_key()))
        .multipart(form)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        return Err(anyhow::anyhow!("Groq STT {}: {}", status, body));
    }

    #[derive(serde::Deserialize)]
    struct GroqTranscription {
        text: String,
    }

    let result: GroqTranscription = resp.json().await?;

    println!("Groq text : {}", result.text);

    Ok(result.text)
}
