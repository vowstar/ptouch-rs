use std::net::SocketAddr;
use std::time::Duration;

use snmp2::{Oid, SyncSession, Value};

use crate::{PtouchError, Result, status::PrinterStatus};

const BROTHER_STATUS_OID: [u64; 14] = [1, 3, 6, 1, 4, 1, 2435, 3, 3, 9, 1, 6, 1, 0];
const PT_P900W_SERIES: u8 = 0x30;
const PT_P900W_MODEL: u8 = 0x6F;
const SNMP_TIMEOUT: Duration = Duration::from_secs(2);

/// Query the Brother status OID for a network-connected PT-P900W.
pub fn query_p900w_status(address: SocketAddr, community: &[u8]) -> Result<PrinterStatus> {
    query_p900w_status_at(SocketAddr::new(address.ip(), 161), community)
}

pub fn community_from_environment() -> Vec<u8> {
    std::env::var("PTOUCH_SNMP_COMMUNITY")
        .unwrap_or_else(|_| "public".to_string())
        .into_bytes()
}

fn query_p900w_status_at(address: SocketAddr, community: &[u8]) -> Result<PrinterStatus> {
    let oid = Oid::from(&BROTHER_STATUS_OID[..])
        .map_err(|_| PtouchError::SnmpError("Invalid built-in Brother status OID".into()))?;
    let mut session = SyncSession::new_v2c(address, community, Some(SNMP_TIMEOUT), 0)
        .map_err(|error| PtouchError::SnmpError(error.to_string()))?;
    let mut response = session
        .get(&oid)
        .map_err(|error| PtouchError::SnmpError(error.to_string()))?;
    if response.error_status != 0 {
        return Err(PtouchError::SnmpError(format!(
            "Brother status OID returned error status {}",
            response.error_status
        )));
    }

    let Some((response_oid, Value::OctetString(bytes))) = response.varbinds.next() else {
        return Err(PtouchError::SnmpError(
            "Brother status OID did not return an octet string".into(),
        ));
    };
    if response_oid != oid {
        return Err(PtouchError::SnmpError(
            "SNMP response returned a different status OID".into(),
        ));
    }

    decode_p900w_status(bytes)
}

fn decode_p900w_status(bytes: &[u8]) -> Result<PrinterStatus> {
    let status = PrinterStatus::from_bytes(bytes)
        .ok_or_else(|| PtouchError::StatusError("SNMP status must contain 32 bytes".into()))?;
    if status.print_head_mark != 0x80
        || status.size != 0x20
        || status.brother_code != 0x42
        || status.series_code != PT_P900W_SERIES
        || status.model_code != PT_P900W_MODEL
    {
        return Err(PtouchError::StatusError(
            "SNMP status is not from a PT-P900W".into(),
        ));
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::UdpSocket, thread};

    #[test]
    fn parses_p900w_media_status_from_the_brother_snmp_oid() {
        let socket = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
        let address = socket.local_addr().unwrap();
        let responder = thread::spawn(move || {
            let mut request = [0; 512];
            let (length, peer) = socket.recv_from(&mut request).unwrap();
            let request = &request[..length];
            let oid = Oid::from(&BROTHER_STATUS_OID[..]).unwrap();
            assert!(
                request
                    .windows(oid.as_bytes().len())
                    .any(|window| window == oid.as_bytes())
            );
            let response =
                snmp_response(request_id(request), &BROTHER_STATUS_OID, &status_packet());
            socket.send_to(&response, peer).unwrap();
        });

        let status = query_p900w_status_at(address, b"public").unwrap();
        assert_eq!(status.media_width, 24);
        assert_eq!(status.media_type, 0x01);
        assert_eq!(status.text_color, 0x08);
        responder.join().unwrap();
    }

    #[test]
    fn rejects_other_models_and_malformed_status_frames() {
        let mut packet = status_packet();
        packet[4] = 0x66;
        assert!(matches!(
            decode_p900w_status(&packet),
            Err(PtouchError::StatusError(_))
        ));
        assert!(matches!(
            decode_p900w_status(&[0; 31]),
            Err(PtouchError::StatusError(_))
        ));
    }

    fn status_packet() -> [u8; 32] {
        let mut packet = [0; 32];
        packet[0] = 0x80;
        packet[1] = 0x20;
        packet[2] = 0x42;
        packet[3] = PT_P900W_SERIES;
        packet[4] = PT_P900W_MODEL;
        packet[10] = 24;
        packet[11] = 0x01;
        packet[25] = 0x08;
        packet
    }

    fn request_id(packet: &[u8]) -> i32 {
        let pdu = packet.iter().position(|byte| *byte == 0xA0).unwrap();
        let content = pdu + 2;
        assert_eq!(packet[content], 0x02);
        let integer_len = usize::from(packet[content + 1]);
        packet[content + 2..content + 2 + integer_len]
            .iter()
            .fold(0, |value, byte| (value << 8) | i32::from(*byte))
    }

    fn snmp_response(request_id: i32, oid: &[u64], payload: &[u8]) -> Vec<u8> {
        let mut oid_content = vec![(oid[0] * 40 + oid[1]) as u8];
        for arc in &oid[2..] {
            let mut value = *arc;
            let mut encoded = vec![(value & 0x7F) as u8];
            value >>= 7;
            while value != 0 {
                encoded.push(((value & 0x7F) as u8) | 0x80);
                value >>= 7;
            }
            encoded.reverse();
            oid_content.extend(encoded);
        }

        let varbind = tlv(
            0x30,
            &[tlv(0x06, &oid_content), tlv(0x04, payload)].concat(),
        );
        let varbinds = tlv(0x30, &varbind);
        let pdu = tlv(
            0xA2,
            &[
                tlv(0x02, &integer_content(request_id)),
                tlv(0x02, &[0]),
                tlv(0x02, &[0]),
                varbinds,
            ]
            .concat(),
        );
        tlv(0x30, &[tlv(0x02, &[1]), tlv(0x04, b"public"), pdu].concat())
    }

    fn integer_content(value: i32) -> Vec<u8> {
        let mut bytes = value.to_be_bytes().to_vec();
        while bytes.len() > 1 && bytes[0] == 0 && bytes[1] & 0x80 == 0 {
            bytes.remove(0);
        }
        bytes
    }

    fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut result = vec![tag, u8::try_from(content.len()).unwrap()];
        result.extend_from_slice(content);
        result
    }
}
