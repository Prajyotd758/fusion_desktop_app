// serial_discovery.rs
// Finds the AirGrip device's COM port automatically instead of a hardcoded port string.
use serialport::{SerialPortInfo, SerialPortType};
/// Confirmed from device output: ESP32-S3 native USB CDC.
/// VID 0x303a = Espressif Systems, PID 0x1001 = ESP32-S3 default USB CDC PID.
const AIRGRIP_VID: u16 = 0x303a;
const AIRGRIP_PID: u16 = 0x1001;

fn is_airgrip_port(port: &SerialPortInfo) -> bool {
    if let SerialPortType::UsbPort(info) = &port.port_type {
        if info.vid == AIRGRIP_VID && info.pid == AIRGRIP_PID {
            return true;
        }
    }
    false
}

/// Returns the port name (e.g. "COM15") for the connected AirGrip device, if found.
pub fn find_airgrip_port() -> Option<String> {
    let ports = serialport::available_ports().ok()?;
    ports.into_iter().find(is_airgrip_port).map(|p| p.port_name)
}
