use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::*;
use crate::status::STATUS_PACKET_SIZE;

struct VirtualPrinter {
    address: SocketAddr,
    worker: JoinHandle<ObservedSession>,
}

struct ObservedSession {
    init: Vec<u8>,
    status_request: [u8; 3],
    print_job: Vec<u8>,
}

impl VirtualPrinter {
    fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut init = vec![0; 102];
            stream.read_exact(&mut init).unwrap();
            let mut status_request = [0; 3];
            stream.read_exact(&mut status_request).unwrap();
            stream.write_all(&status_packet(0x00)).unwrap();
            let mut print_job = Vec::new();
            loop {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                print_job.push(byte[0]);
                if byte[0] == 0x1A {
                    break;
                }
            }
            stream.write_all(&status_packet(0x01)).unwrap();
            let mut close_probe = [0; 1];
            let _ = stream.read(&mut close_probe);
            ObservedSession {
                init,
                status_request,
                print_job,
            }
        });
        Self { address, worker }
    }

    fn address(&self) -> SocketAddr {
        self.address
    }

    fn finish(self) -> ObservedSession {
        self.worker.join().unwrap()
    }
}

fn status_packet(status_type: u8) -> [u8; STATUS_PACKET_SIZE] {
    let mut packet = [0; STATUS_PACKET_SIZE];
    packet[0] = 0x80;
    packet[1] = 0x20;
    packet[10] = 24;
    packet[11] = 0x01;
    packet[18] = status_type;
    packet
}

fn standard_job(width: usize, packbits: bool) -> Vec<u8> {
    let mut job = vec![
        0x1B, 0x69, 0x61, 0x01, 0x1B, 0x69, 0x7A, 0x00, 0x00, 24, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ];
    if packbits {
        job.extend([0x4D, 0x02, 0x47, (width + 1) as u8, 0x00, (width - 1) as u8]);
    } else {
        job.extend([0x47, width as u8, 0x00]);
    }
    job.extend(vec![0xAA; width]);
    job.push(0x1A);
    job
}

fn high_resolution_job(width: usize, packbits: bool, margin_dots: u8) -> Vec<u8> {
    let mut job = vec![
        0x1B,
        0x69,
        0x61,
        0x01,
        0x1B,
        0x69,
        0x7A,
        0x00,
        0x00,
        24,
        0x00,
        0x02,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x1B,
        0x69,
        0x4B,
        0x4C,
        0x1B,
        0x69,
        0x64,
        margin_dots,
        0x00,
    ];
    if packbits {
        job.extend([0x4D, 0x02]);
        for _ in 0..2 {
            job.extend([0x47, (width + 1) as u8, 0x00, (width - 1) as u8]);
            job.extend(vec![0xAA; width]);
        }
    } else {
        for _ in 0..2 {
            job.extend([0x47, width as u8, 0x00]);
            job.extend(vec![0xAA; width]);
        }
    }
    job.push(0x1A);
    job
}

#[test]
fn target_standard_jobs_match_golden_bytes_through_virtual_printer() {
    for (pid, model, width, packbits) in [
        (0x2060, "PT-E550W", 16, true),
        (0x2085, "PT-P900W", 70, false),
    ] {
        let expected_job = standard_job(width, packbits);
        let printer = VirtualPrinter::start();
        let mut device = PtouchDevice::open_tcp(printer.address(), pid).unwrap();
        assert_eq!(device.device_info().name, model);
        device.init().unwrap();
        device
            .print_raster(
                &[vec![0xAA; width]],
                false,
                false,
                protocol::PrintQuality::Standard,
            )
            .unwrap();
        device.close().unwrap();

        let observed = printer.finish();
        let mut expected_init = vec![0; 100];
        expected_init.extend([0x1B, 0x40]);
        assert_eq!(observed.init, expected_init);
        assert_eq!(observed.status_request, [0x1B, 0x69, 0x53]);
        assert_eq!(observed.print_job, expected_job);
    }
}

#[test]
fn target_high_resolution_jobs_match_golden_bytes_through_virtual_printer() {
    for (pid, width, packbits, margin_dots) in [(0x2060, 16, true, 28), (0x2085, 70, false, 56)] {
        let printer = VirtualPrinter::start();
        let mut device = PtouchDevice::open_tcp(printer.address(), pid).unwrap();
        device.init().unwrap();
        device
            .print_raster(
                &[vec![0xAA; width]],
                false,
                false,
                protocol::PrintQuality::HighRes,
            )
            .unwrap();
        device.close().unwrap();

        let observed = printer.finish();
        assert_eq!(
            observed.print_job,
            high_resolution_job(width, packbits, margin_dots)
        );
    }
}
