use anyhow::Result;
use reqwest::multipart;
use std::fs;

pub async fn transcribe() -> Result<String> {
    let client = reqwest::Client::new();

    let audio = fs::read("../../data/audio/latest.wav")?;

    let part = multipart::Part::bytes(audio)
        .file_name("latest.wav")
        .mime_str("audio/wav")?;

    let form = multipart::Form::new()
        .part("file", part)
        .text("language", "auto") // <-- was missing entirely
        .text("temperature", "0.0");
    let response = client
        .post("http://127.0.0.1:8080/inference")
        .multipart(form)
        .send()
        .await?;

    let status = response.status();
    let raw = response.text().await?;

    println!("status: {}", status);
    println!("raw body: {}", raw);

    let json: serde_json::Value = serde_json::from_str(&raw)?;
    let text = json["text"].as_str().unwrap_or("").trim().to_string();

    println!("text : {}", text);
    Ok(text)
}
