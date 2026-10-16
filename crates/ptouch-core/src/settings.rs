use crate::{error::PtouchError, transport::PtouchDevice};

const COMMAND_MODE_GET: [u8; 5] = [0x1B, 0x69, 0x55, 0x61, 0x01];
const PT_P900W_PID: u16 = 0x2085;
const PT_P900W_SERIES: u8 = 0x30;
const PT_P900W_MODEL: u8 = 0x6F;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Command Mode values returned by the PT-P900W settings GET command.
pub enum CommandMode {
    EscP,
    Raster,
    PTouchTemplate,
    Unknown(u8),
}

impl CommandMode {
    fn from_raw(value: u8) -> Self {
        match value {
            0x00 => Self::EscP,
            0x01 => Self::Raster,
            0x03 => Self::PTouchTemplate,
            value => Self::Unknown(value),
        }
    }
}

impl std::fmt::Display for CommandMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EscP => formatter.write_str("ESC/P"),
            Self::Raster => formatter.write_str("Raster"),
            Self::PTouchTemplate => formatter.write_str("P-touch Template"),
            Self::Unknown(value) => write!(formatter, "Unknown (0x{value:02X})"),
        }
    }
}

/// Read the PT-P900W command mode using its recovered, read-only GET command.
pub fn query_command_mode(device: &PtouchDevice) -> crate::Result<CommandMode> {
    if device.device_info().pid != PT_P900W_PID {
        return Err(PtouchError::UnsupportedOperation(
            "Command Mode query is supported only on PT-P900W",
        ));
    }

    device.send(&COMMAND_MODE_GET)?;
    let mut reply = [0; crate::status::STATUS_PACKET_SIZE];
    let mut received = 0;
    while received < reply.len() {
        let count = device.receive(&mut reply[received..])?;
        if count == 0 {
            return Err(PtouchError::StatusError(
                "Command Mode reply ended before the status packet was complete".into(),
            ));
        }
        received += count;
    }

    let status = crate::status::PrinterStatus::from_bytes(&reply)
        .ok_or_else(|| PtouchError::StatusError("Invalid Command Mode status reply".into()))?;
    if status.print_head_mark != 0x80
        || status.size != 0x20
        || status.brother_code != 0x42
        || status.series_code != PT_P900W_SERIES
        || status.model_code != PT_P900W_MODEL
    {
        return Err(PtouchError::StatusError(
            "Command Mode reply is not from a PT-P900W".into(),
        ));
    }

    Ok(CommandMode::from_raw(status.hw_settings_5))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Duration,
    };

    fn reply(mode: u8) -> [u8; 32] {
        let mut packet = [0; 32];
        packet[0] = 0x80;
        packet[1] = 0x20;
        packet[2] = 0x42;
        packet[3] = 0x30;
        packet[4] = 0x6F;
        packet[30] = mode;
        packet
    }

    fn query(mode: u8) -> (CommandMode, Vec<u8>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 5];
            stream.read_exact(&mut request).unwrap();
            stream.write_all(&reply(mode)).unwrap();
            let mut close_probe = [0; 1];
            let _ = stream.read(&mut close_probe);
            request.to_vec()
        });

        let device = PtouchDevice::open_tcp(address, 0x2085).unwrap();
        let mode = query_command_mode(&device).unwrap();
        device.close().unwrap();
        (mode, worker.join().unwrap())
    }

    #[test]
    fn command_mode_query_sends_only_the_recovered_read_request() {
        let (mode, request) = query(0x01);
        assert_eq!(request, [0x1B, 0x69, 0x55, 0x61, 0x01]);
        assert_eq!(mode, CommandMode::Raster);
    }

    #[test]
    fn command_mode_query_preserves_unknown_values() {
        let (mode, _) = query(0x7F);
        assert_eq!(mode, CommandMode::Unknown(0x7F));
    }

    #[test]
    fn command_mode_displays_documented_values_and_raw_unknowns() {
        assert_eq!(CommandMode::EscP.to_string(), "ESC/P");
        assert_eq!(CommandMode::Raster.to_string(), "Raster");
        assert_eq!(CommandMode::PTouchTemplate.to_string(), "P-touch Template");
        assert_eq!(CommandMode::Unknown(0x7F).to_string(), "Unknown (0x7F)");
    }

    #[test]
    fn command_mode_query_rejects_non_p900w_models_before_sending() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 5];
            stream.read(&mut request).unwrap_or(0)
        });

        let device = PtouchDevice::open_tcp(address, 0x2060).unwrap();
        assert!(matches!(
            query_command_mode(&device),
            Err(PtouchError::UnsupportedOperation(_))
        ));
        device.close().unwrap();
        assert_eq!(worker.join().unwrap(), 0);
    }
}
