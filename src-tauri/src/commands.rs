use crate::whisper;

pub async fn run_transcribe_only() -> Result<String, String> {
    let text = whisper::transcribe().await.map_err(|e| e.to_string())?;
    println!("Transcript: {}", text);

    Ok(text)
}
