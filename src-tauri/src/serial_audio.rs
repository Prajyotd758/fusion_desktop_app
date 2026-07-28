use serialport::SerialPort;
use std::io::Read;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const MAGIC_1: u8 = 0xAB;
const MAGIC_2: u8 = 0xCD;

pub fn start_serial_listener(app: AppHandle, port_name: String) {
    std::thread::spawn(move || {
        let port = serialport::new(&port_name, 921_600)
            .timeout(Duration::from_millis(1000))
            .open();

        let mut port = match port {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to open serial port {}: {}", port_name, e);
                return;
            }
        };

        let mut byte_buf = [0u8; 1];
        let mut state = 0u8; // 0=looking for 0xAB, 1=found 0xAB, 2=have len bytes
        let mut len_bytes = [0u8; 2];
        let mut len_idx = 0;
        let mut payload_len: usize = 0;
        let mut payload: Vec<u8> = Vec::new();

        loop {
            match port.read(&mut byte_buf) {
                Ok(0) => continue,
                Ok(_) => {
                    let b = byte_buf[0];

                    match state {
                        0 => {
                            if b == MAGIC_1 {
                                state = 1;
                            }
                        }
                        1 => {
                            if b == MAGIC_2 {
                                state = 2;
                                len_idx = 0;
                            } else {
                                state = 0;
                            }
                        }
                        2 => {
                            len_bytes[len_idx] = b;
                            len_idx += 1;
                            if len_idx == 2 {
                                payload_len =
                                    (len_bytes[0] as usize) | ((len_bytes[1] as usize) << 8);
                                payload.clear();
                                state = if payload_len == 0 { 0 } else { 3 };
                            }
                        }
                        3 => {
                            payload.push(b);
                            if payload.len() == payload_len {
                                // Convert bytes to i16 samples
                                let samples: Vec<i16> = payload
                                    .chunks_exact(2)
                                    .map(|c| i16::from_le_bytes([c[0], c[1]]))
                                    .collect();

                                let _ = app.emit("audio-chunk", samples);
                                state = 0;
                            }
                        }
                        _ => state = 0,
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(e) => {
                    eprintln!("Serial read error: {}", e);
                    break;
                }
            }
        }
    });
}
