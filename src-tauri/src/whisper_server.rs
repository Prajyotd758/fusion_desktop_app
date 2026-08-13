use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::OnceLock;

static WHISPER_SERVER: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
static LLAMA_SERVER: OnceLock<Mutex<Option<Child>>> = OnceLock::new();

pub fn start_whisper_server() {
    let server = WHISPER_SERVER.get_or_init(|| Mutex::new(None));
    let mut server = server.lock().unwrap();

    if server.is_none() {
        match Command::new("../../whisper.cpp/build/bin/Release/whisper-server.exe")
            .args([
                "-m",
                "../../models/ggml-base.bin",
                "--host",
                "127.0.0.1",
                "--port",
                "8080",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                *server = Some(child);
                println!("Whisper server started on port 8080");
            }
            Err(e) => eprintln!("Failed to start whisper server: {e}"),
        }
    }
}

pub fn start_llama_server() {
    let server = LLAMA_SERVER.get_or_init(|| Mutex::new(None));
    let mut server = server.lock().unwrap();

    if server.is_none() {
        match Command::new("../../llama.cpp/build-vulkan/bin/Release/llama-server.exe")
            .args([
                "-m",
                "../../models/qwen2.5-3b-instruct-q4_K_M.gguf",
                "--port",
                "8081",
                "-c",
                "2048",
                "-ngl",
                "20",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                *server = Some(child);
                println!("Llama server started on port 8081");
            }
            Err(e) => eprintln!("Failed to start llama server: {e}"),
        }
    }
}
