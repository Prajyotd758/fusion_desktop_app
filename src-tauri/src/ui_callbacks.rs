#[tauri::command]
pub async fn check_internet() -> bool {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();

    client
        .head("https://www.google.com")
        .send()
        .await
        .map(|res| res.status().is_success() || res.status().is_redirection())
        .unwrap_or(false)
}