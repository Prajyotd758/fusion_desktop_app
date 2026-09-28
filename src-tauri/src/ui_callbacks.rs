#[tauri::command]
pub async fn check_internet() -> bool {
    crate::system::helper_functions::http_client()
        .head("https://api.groq.com")
        .send()
        .await
        .is_ok()
}
