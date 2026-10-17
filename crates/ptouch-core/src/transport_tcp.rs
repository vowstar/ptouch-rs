// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use crate::session::Transport;
use crate::{CancellationToken, PtouchError, Result};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const IO_POLL_TIMEOUT: Duration = Duration::from_millis(250);

pub(super) struct TcpTransport {
    stream: TcpStream,
    address: SocketAddr,
    cancellation: CancellationToken,
}

impl TcpTransport {
    pub(super) fn connect(address: SocketAddr) -> Result<Self> {
        let stream =
            TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).map_err(map_tcp_error)?;
        stream.set_nodelay(true).map_err(map_tcp_error)?;
        Ok(Self {
            stream,
            address,
            cancellation: CancellationToken::default(),
        })
    }

    pub(super) fn address(&self) -> SocketAddr {
        self.address
    }

    pub(super) fn set_cancellation(&mut self, token: CancellationToken) {
        self.cancellation = token;
    }
}

impl Transport for TcpTransport {
    fn send(&self, data: &[u8], timeout: Duration) -> Result<()> {
        let started = Instant::now();
        let mut offset = 0;
        while offset < data.len() {
            self.cancellation.check()?;
            let remaining = remaining_timeout(timeout, started.elapsed())?;
            self.stream
                .set_write_timeout(Some(remaining.min(IO_POLL_TIMEOUT)))
                .map_err(map_tcp_error)?;
            match (&self.stream).write(&data[offset..]) {
                Ok(0) => {
                    return Err(PtouchError::SendFailed(
                        "TCP socket accepted zero bytes".to_owned(),
                    ));
                }
                Ok(written) => offset += written,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if is_timeout(&error) => {}
                Err(error) => return Err(map_tcp_error(error)),
            }
        }
        Ok(())
    }

    fn receive(&self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        let started = Instant::now();
        loop {
            self.cancellation.check()?;
            let remaining = remaining_timeout(timeout, started.elapsed())?;
            self.stream
                .set_read_timeout(Some(remaining.min(IO_POLL_TIMEOUT)))
                .map_err(map_tcp_error)?;
            match (&self.stream).read(buf) {
                Ok(read) => return Ok(read),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if is_timeout(&error) => {}
                Err(error) => return Err(map_tcp_error(error)),
            }
        }
    }

    fn close(self) -> Result<()> {
        self.stream.shutdown(Shutdown::Both).map_err(map_tcp_error)
    }
}

fn remaining_timeout(timeout: Duration, elapsed: Duration) -> Result<Duration> {
    timeout
        .checked_sub(elapsed)
        .filter(|remaining| !remaining.is_zero())
        .ok_or(PtouchError::Timeout)
}

fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

fn map_tcp_error(error: io::Error) -> PtouchError {
    if is_timeout(&error) {
        PtouchError::Timeout
    } else {
        PtouchError::TcpError(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    use crate::transport::PtouchDevice;

    #[test]
    fn tcp_transport_preserves_partial_reads_and_reports_deadline_timeout() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let (accepted_tx, accepted_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            accepted_tx.send(()).unwrap();
            stream.write_all(&[0x41]).unwrap();
            stream.write_all(&[0x42]).unwrap();
            let mut close_probe = [0; 1];
            let _ = stream.read(&mut close_probe);
        });

        let transport = TcpTransport::connect(address).unwrap();
        accepted_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let mut first = [0; 1];
        assert_eq!(
            transport
                .receive(&mut first, Duration::from_secs(1))
                .unwrap(),
            1
        );
        assert_eq!(first, [0x41]);
        let mut second = [0; 1];
        assert_eq!(
            transport
                .receive(&mut second, Duration::from_millis(20))
                .unwrap(),
            1
        );
        assert_eq!(second, [0x42]);
        let mut empty = [0; 1];
        assert!(matches!(
            transport.receive(&mut empty, Duration::from_millis(20)),
            Err(PtouchError::Timeout)
        ));
        transport.close().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn tcp_transport_cancellation_stops_send_and_receive_before_io() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut probe = [0; 1];
            stream.read(&mut probe).unwrap()
        });

        let mut transport = TcpTransport::connect(address).unwrap();
        let cancellation = CancellationToken::default();
        transport.set_cancellation(cancellation.clone());
        cancellation.cancel();

        assert!(matches!(
            transport.send(&[0xAA], Duration::from_secs(1)),
            Err(PtouchError::Cancelled)
        ));
        let mut response = [0; 1];
        assert!(matches!(
            transport.receive(&mut response, Duration::from_secs(1)),
            Err(PtouchError::Cancelled)
        ));
        transport.close().unwrap();
        assert_eq!(server.join().unwrap(), 0);
    }

    #[test]
    fn tcp_device_selects_target_model_and_round_trips_raw_bytes() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4];
                stream.read_exact(&mut request).unwrap();
                stream.write_all(&[5, 6, 7]).unwrap();
                requests.push(request);
            }
            requests
        });

        for (pid, name) in [(0x2060, "PT-E550W"), (0x2085, "PT-P900W")] {
            let device = PtouchDevice::open_tcp(address, pid).unwrap();
            assert_eq!(device.device_info().name, name);
            device.send(&[1, 2, 3, 4]).unwrap();
            let mut response = [0; 3];
            assert_eq!(device.receive(&mut response).unwrap(), response.len());
            assert_eq!(response, [5, 6, 7]);
            device.close().unwrap();
        }

        assert_eq!(server.join().unwrap(), [[1, 2, 3, 4], [1, 2, 3, 4]]);
    }

    #[test]
    fn tcp_initialization_does_not_wait_for_an_unsupported_raw_status_reply() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut init = [0; 102];
            stream.read_exact(&mut init).unwrap();
            let mut remainder = Vec::new();
            stream.read_to_end(&mut remainder).unwrap();
            (init, remainder)
        });

        let mut device = PtouchDevice::open_tcp(address, 0x2085).unwrap();
        device.init().unwrap();
        device.close().unwrap();

        let (init, remainder) = server.join().unwrap();
        assert!(init[..100].iter().all(|byte| *byte == 0));
        assert_eq!(&init[100..], &[0x1B, 0x40]);
        assert!(remainder.is_empty());
    }

    #[test]
    fn tcp_device_rejects_unknown_model_before_connecting() {
        let address = "127.0.0.1:1".parse().unwrap();

        assert!(matches!(
            PtouchDevice::open_tcp(address, 0xffff),
            Err(PtouchError::DeviceNotFound)
        ));
    }

    #[test]
    fn tcp_socket_errors_preserve_timeout_and_connection_failure_types() {
        assert!(matches!(
            map_tcp_error(io::Error::from(io::ErrorKind::TimedOut)),
            PtouchError::Timeout
        ));
        assert!(matches!(
            map_tcp_error(io::Error::from(io::ErrorKind::ConnectionRefused)),
            PtouchError::TcpError(error)
                if error.kind() == io::ErrorKind::ConnectionRefused
        ));
    }
}
