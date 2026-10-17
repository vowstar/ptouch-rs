// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Background worker for non-blocking USB and Bluetooth printer operations.

#[cfg(target_os = "macos")]
use std::io::Write;
use std::net::SocketAddr;
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use log::{error, info};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use ptouch_core::device::DeviceFlags;
use ptouch_core::protocol::PrintQuality;
use ptouch_core::transport::PtouchDevice;

use crate::state::{
    NetworkPrinterCandidate, PrinterCommand, PrinterEvent, PrinterResponse, PrinterTarget,
};

const POLL_INTERVAL: Duration = Duration::from_secs(3);
const NETWORK_DISCOVERY_WINDOW: Duration = Duration::from_secs(3);
const RAW_PRINTER_SERVICE: &str = "_pdl-datastream._tcp.local.";
const HELPER_ARG: &str = "--ptouch-bluetooth-helper";

struct PrintJob<'a> {
    target: &'a PrinterTarget,
    raster_lines: &'a [Vec<u8>],
    chain_print: bool,
    auto_cut: bool,
    quality: PrintQuality,
    tape_width_mm: u8,
    cancellation: &'a ptouch_core::CancellationToken,
}

pub fn printer_worker(
    cmd_rx: mpsc::Receiver<PrinterCommand>,
    resp_tx: mpsc::Sender<PrinterEvent>,
    ctx: egui::Context,
) {
    info!("Printer worker started");
    let mut current_target = PrinterTarget::Usb;
    // An interrupted job can still be printing. Resume status requests only
    // after an explicit user operation, not the background polling timer.
    let mut poll_enabled = true;
    discover_bluetooth(&resp_tx, &ctx);
    discover_usb(&resp_tx, &ctx);
    discover_network(&resp_tx, &ctx);

    loop {
        match cmd_rx.recv_timeout(POLL_INTERVAL) {
            Ok(PrinterCommand::DiscoverBluetooth) => discover_bluetooth(&resp_tx, &ctx),
            Ok(PrinterCommand::DiscoverNetwork) => discover_network(&resp_tx, &ctx),
            Ok(PrinterCommand::Poll(target)) => {
                discover_usb(&resp_tx, &ctx);
                current_target = target;
                poll_enabled = true;
                do_poll(&current_target, &resp_tx, &ctx);
            }
            Ok(PrinterCommand::Print {
                raster_lines,
                chain_print,
                auto_cut,
                quality,
                tape_width_mm,
                target,
                cancellation,
            }) => {
                current_target = target;
                poll_enabled = do_print(
                    PrintJob {
                        target: &current_target,
                        raster_lines: &raster_lines,
                        chain_print,
                        auto_cut,
                        quality,
                        tape_width_mm,
                        cancellation: &cancellation,
                    },
                    (&resp_tx, &ctx),
                );
            }
            Ok(PrinterCommand::FeedAndCut {
                target,
                cancellation,
            }) => {
                current_target = target;
                poll_enabled = do_feed_and_cut(&current_target, &resp_tx, &ctx, cancellation);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if poll_enabled && !current_target.is_bluetooth() {
                    do_poll(&current_target, &resp_tx, &ctx);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn discover_usb(tx: &mpsc::Sender<PrinterEvent>, ctx: &egui::Context) {
    let devices: Vec<_> = ptouch_core::diagnostics::doctor(false)
        .devices
        .into_iter()
        .filter(|device| device.model.is_some())
        .map(|device| PrinterTarget::UsbAt(device.location))
        .collect();
    #[cfg(windows)]
    let devices = {
        let mut devices = devices;
        match ptouch_core::usbprint::list() {
            Ok(printers) => devices.extend(printers.into_iter().map(|p| PrinterTarget::UsbPrint {
                instance_id: p.instance_id,
            })),
            Err(error) => error!("USBPRINT discovery failed: {error}"),
        }
        devices
    };
    let _ = tx.send(PrinterEvent {
        target: None,
        response: PrinterResponse::UsbDevices(devices),
    });
    ctx.request_repaint();
}

fn discover_bluetooth(resp_tx: &mpsc::Sender<PrinterEvent>, ctx: &egui::Context) {
    #[cfg(target_os = "macos")]
    let devices = run_helper(&["list"], None)
        .map(|output| parse_bluetooth_devices(&output))
        .unwrap_or_else(|message| {
            error!("Bluetooth discovery failed: {message}");
            Vec::new()
        });
    #[cfg(not(target_os = "macos"))]
    let devices = Vec::new();

    let _ = resp_tx.send(PrinterEvent {
        target: None,
        response: PrinterResponse::BluetoothDevices(devices),
    });
    ctx.request_repaint();
}

fn discover_network(tx: &mpsc::Sender<PrinterEvent>, ctx: &egui::Context) {
    let devices = match browse_raw_printers() {
        Ok(devices) => devices,
        Err(message) => {
            error!("Network printer discovery failed: {message}");
            Vec::new()
        }
    };
    let _ = tx.send(PrinterEvent {
        target: None,
        response: PrinterResponse::NetworkDevices(devices),
    });
    ctx.request_repaint();
}

fn browse_raw_printers() -> Result<Vec<NetworkPrinterCandidate>, String> {
    let daemon = ServiceDaemon::new().map_err(|error| error.to_string())?;
    let receiver = daemon
        .browse(RAW_PRINTER_SERVICE)
        .map_err(|error| error.to_string())?;
    let started = Instant::now();
    let mut candidates = Vec::new();

    while let Some(remaining) = NETWORK_DISCOVERY_WINDOW.checked_sub(started.elapsed()) {
        match receiver.recv_timeout(remaining) {
            Ok(ServiceEvent::ServiceResolved(service))
                if service.is_valid() && service.port > 0 =>
            {
                let name = service
                    .fullname
                    .strip_suffix("._pdl-datastream._tcp.local.")
                    .unwrap_or(&service.fullname)
                    .to_string();
                let pid = ["usb_MDL", "ty", "product"]
                    .into_iter()
                    .filter_map(|key| service.get_property_val_str(key))
                    .find_map(model_pid_from_service_text);
                let mut addresses: Vec<_> = service.get_addresses_v4().into_iter().collect();
                addresses.sort_unstable();
                for ip in addresses {
                    let candidate = NetworkPrinterCandidate {
                        name: name.clone(),
                        address: SocketAddr::from((ip, service.port)),
                        pid,
                    };
                    if !candidates.iter().any(|existing: &NetworkPrinterCandidate| {
                        existing.address == candidate.address
                    }) {
                        candidates.push(candidate);
                    }
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }

    let _ = daemon.stop_browse(RAW_PRINTER_SERVICE);
    let _ = daemon.shutdown();
    candidates.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.address.cmp(&right.address))
    });
    Ok(candidates)
}

fn model_pid_from_service_text(value: &str) -> Option<u16> {
    let normalized = value.to_ascii_lowercase();
    if normalized.contains("pt-p900w") {
        Some(0x2085)
    } else if normalized.contains("pt-e550w") {
        Some(0x2060)
    } else {
        None
    }
}

fn do_poll(target: &PrinterTarget, tx: &mpsc::Sender<PrinterEvent>, ctx: &egui::Context) {
    let response = match target {
        PrinterTarget::Usb | PrinterTarget::UsbAt(_) | PrinterTarget::Tcp { .. } => {
            poll_printer(target)
        }
        #[cfg(windows)]
        PrinterTarget::UsbPrint { .. } => poll_printer(target),
        #[cfg(any(target_os = "macos", test))]
        PrinterTarget::Bluetooth { address, .. } => poll_bluetooth(address),
    }
    .unwrap_or_else(|message| {
        error!("Poll failed: {message}");
        PrinterResponse::ConnectionError(message)
    });
    let _ = tx.send(PrinterEvent {
        target: Some(target.clone()),
        response,
    });
    ctx.request_repaint();
}

fn open_printer(target: &PrinterTarget) -> ptouch_core::Result<PtouchDevice> {
    if let PrinterTarget::Tcp { address, pid } = target {
        return PtouchDevice::open_tcp(*address, *pid);
    }
    #[cfg(windows)]
    if let PrinterTarget::UsbPrint { instance_id } = target {
        return PtouchDevice::open_usbprint(instance_id);
    }
    target
        .usb_location()
        .map_or_else(PtouchDevice::open_first, PtouchDevice::open_at)
}

fn poll_printer(target: &PrinterTarget) -> Result<PrinterResponse, String> {
    let mut dev = open_printer(target).map_err(|e| e.to_string())?;
    let max_px = dev.max_px();
    let dpi = dev.device_info().dpi;
    let flags = dev.flags();
    let status = if dev.is_tcp() && !flags.contains(DeviceFlags::SNMP_STATUS) {
        None
    } else {
        Some(dev.query_status().map_err(|e| e.to_string())?.clone())
    };
    let media_status_available = status.is_some();
    let media_detected = status.as_ref().is_some_and(|status| status.media_width > 0);
    let quality_modes = flags.contains(DeviceFlags::LEGACY_HIRES)
        || (flags.contains(DeviceFlags::ADVANCED_HIRES)
            && status
                .as_ref()
                .is_some_and(|status| status.media_type == 0x01 && status.text_color == 0x08));
    let draft_quality_modes = flags.contains(DeviceFlags::LEGACY_HIRES);
    let model_name = dev.device_info().name.to_string();
    let media_width = status.as_ref().map_or(0, |status| status.media_width);
    let media_type = status.as_ref().map_or_else(
        || "Status unavailable".to_string(),
        |status| status.media_type_name().to_string(),
    );
    let tape_width_px = dev.tape_width_px().unwrap_or(max_px);
    let response = PrinterResponse::Connected {
        model_name,
        media_width,
        media_type,
        media_status_available,
        media_detected,
        max_px,
        dpi,
        quality_modes,
        draft_quality_modes,
        tape_width_px,
    };
    let _ = dev.close();
    Ok(response)
}

#[cfg(any(target_os = "macos", test))]
fn poll_bluetooth(address: &str) -> Result<PrinterResponse, String> {
    #[cfg(target_os = "macos")]
    {
        let output = run_helper(&["status", address], None)?;
        parse_bluetooth_status(&output)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = address;
        Err("Bluetooth is available on macOS only".to_string())
    }
}

#[cfg(any(target_os = "macos", test))]
fn parse_bluetooth_devices(output: &str) -> Vec<PrinterTarget> {
    output
        .lines()
        .filter_map(|line| {
            let (address, name) = line.split_once('\t')?;
            Some(PrinterTarget::Bluetooth {
                name: name.to_string(),
                address: address.to_string(),
            })
        })
        .collect()
}

#[cfg(any(target_os = "macos", test))]
fn parse_bluetooth_status(output: &str) -> Result<PrinterResponse, String> {
    let fields: Vec<_> = output.trim_end().split('\t').collect();
    if fields.len() != 6 {
        return Err("Bluetooth helper returned invalid status".to_string());
    }
    Ok(PrinterResponse::Connected {
        model_name: fields[0].to_string(),
        media_width: fields[1].parse().map_err(|_| "Invalid media width")?,
        media_type: fields[2].to_string(),
        media_status_available: true,
        media_detected: true,
        max_px: fields[3].parse().map_err(|_| "Invalid raster width")?,
        dpi: fields[4].parse().map_err(|_| "Invalid resolution")?,
        quality_modes: false,
        draft_quality_modes: false,
        tape_width_px: fields[5].parse().map_err(|_| "Invalid tape width")?,
    })
}

fn do_print(job: PrintJob<'_>, response: (&mpsc::Sender<PrinterEvent>, &egui::Context)) -> bool {
    let (tx, ctx) = response;
    let result = match job.target {
        PrinterTarget::Usb | PrinterTarget::UsbAt(_) | PrinterTarget::Tcp { .. } => {
            print_printer(&job)
        }
        #[cfg(windows)]
        PrinterTarget::UsbPrint { .. } => print_printer(&job),
        #[cfg(any(target_os = "macos", test))]
        PrinterTarget::Bluetooth { address, .. } => print_bluetooth(address, job.raster_lines),
    };
    let completed = result.is_ok();
    let response = result
        .map(|()| PrinterResponse::PrintDone)
        .unwrap_or_else(PrinterResponse::Error);
    let _ = tx.send(PrinterEvent {
        target: Some(job.target.clone()),
        response,
    });
    ctx.request_repaint();
    completed
}

fn print_printer(job: &PrintJob<'_>) -> Result<(), String> {
    let mut dev = open_printer(job.target).map_err(|e| format!("Connect error: {e}"))?;
    dev.set_cancellation_token(job.cancellation.clone());
    dev.init().map_err(|e| format!("Init error: {e}"))?;
    if matches!(job.target, PrinterTarget::Tcp { .. }) {
        if dev.flags().contains(DeviceFlags::SNMP_STATUS) {
            let status = dev
                .query_status()
                .map_err(|e| format!("Status error: {e}"))?;
            if status.media_width == 0 || status.media_width != job.tape_width_mm {
                return Err("Detected tape changed; refresh printer status before printing".into());
            }
        } else {
            dev.set_media_width_override(job.tape_width_mm)
                .map_err(|e| format!("Tape width error: {e}"))?;
        }
    }
    let result = dev
        .print_raster(job.raster_lines, job.chain_print, job.auto_cut, job.quality)
        .map_err(|e| format!("Print error: {e}"));
    let _ = dev.close();
    result
}

#[cfg(any(target_os = "macos", test))]
fn print_bluetooth(address: &str, raster_lines: &[Vec<u8>]) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let bytes: Vec<u8> = raster_lines.iter().flatten().copied().collect();
        run_helper(&["print", address], Some(&bytes)).map(|_| ())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (address, raster_lines);
        Err("Bluetooth is available on macOS only".to_string())
    }
}

fn do_feed_and_cut(
    target: &PrinterTarget,
    tx: &mpsc::Sender<PrinterEvent>,
    ctx: &egui::Context,
    cancellation: ptouch_core::CancellationToken,
) -> bool {
    let result = if target.is_bluetooth() {
        Err("PT-P300BT has a manual cutter".to_string())
    } else {
        (|| {
            let mut dev = open_printer(target).map_err(|e| format!("Connect error: {e}"))?;
            dev.set_cancellation_token(cancellation);
            dev.init().map_err(|e| format!("Init error: {e}"))?;
            let result = dev
                .feed_and_cut()
                .map_err(|e| format!("Feed & cut error: {e}"));
            let _ = dev.close();
            result
        })()
    };
    let completed = result.is_ok();
    let response = result
        .map(|()| PrinterResponse::FeedAndCutDone)
        .unwrap_or_else(PrinterResponse::Error);
    let _ = tx.send(PrinterEvent {
        target: Some(target.clone()),
        response,
    });
    ctx.request_repaint();
    completed
}

#[cfg(target_os = "macos")]
fn run_helper(args: &[&str], stdin: Option<&[u8]>) -> Result<String, String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = Command::new(executable)
        .arg(HELPER_ARG)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(bytes) = stdin {
        child
            .stdin
            .take()
            .ok_or("Cannot open Bluetooth helper input")?
            .write_all(bytes)
            .map_err(|e| e.to_string())?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|e| e.to_string())
    } else {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if message.is_empty() {
            "Bluetooth helper failed".to_string()
        } else {
            message
        })
    }
}

/// Run a Bluetooth operation before eframe claims the process's main thread.
pub fn run_bluetooth_helper_from_args() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some(HELPER_ARG) {
        return None;
    }
    #[cfg(target_os = "macos")]
    let result = run_native_helper(&args[2..]);
    #[cfg(not(target_os = "macos"))]
    let result: Result<(), String> = Err("Bluetooth is available on macOS only".to_string());
    match result {
        Ok(()) => Some(0),
        Err(message) => {
            eprintln!("{message}");
            Some(1)
        }
    }
}

#[cfg(target_os = "macos")]
fn run_native_helper(args: &[String]) -> Result<(), String> {
    use ptouch_core::BluetoothDevice;
    use std::io::Read;

    match args.first().map(String::as_str) {
        Some("list") => {
            for device in BluetoothDevice::paired_devices().map_err(|e| e.to_string())? {
                if device.name.to_ascii_uppercase().starts_with("PT-P300BT") {
                    println!(
                        "{}\t{}",
                        device.address,
                        device.name.replace(['\t', '\n'], " ")
                    );
                }
            }
            Ok(())
        }
        Some("status") => {
            let address = args.get(1).ok_or("Missing Bluetooth address")?;
            let mut device = BluetoothDevice::open(address).map_err(|e| e.to_string())?;
            device.init().map_err(|e| e.to_string())?;
            let status = device.status().ok_or("Printer returned no status")?;
            let tape_width_px = device.tape_width_px().ok_or("Unsupported tape width")?;
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                device.model_name(),
                status.media_width,
                status.media_type_name(),
                device.raster_width_px(),
                device.dpi(),
                tape_width_px
            );
            device.close().map_err(|e| e.to_string())
        }
        Some("print") => {
            let address = args.get(1).ok_or("Missing Bluetooth address")?;
            let mut bytes = Vec::new();
            std::io::stdin()
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            const LINE_BYTES: usize = 16;
            if bytes.is_empty() || bytes.len() % LINE_BYTES != 0 {
                return Err("Invalid PT-P300BT raster data".to_string());
            }
            let lines: Vec<Vec<u8>> = bytes.chunks_exact(LINE_BYTES).map(<[u8]>::to_vec).collect();
            let mut device = BluetoothDevice::open(address).map_err(|e| e.to_string())?;
            device.init().map_err(|e| e.to_string())?;
            device.print_raster(&lines).map_err(|e| e.to_string())?;
            device.close().map_err(|e| e.to_string())
        }
        _ => Err("Unknown Bluetooth helper operation".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn parses_bluetooth_device_list() {
        let devices = parse_bluetooth_devices("EC:79:49:61:CC:F8\tPT-P300BT6427\n");
        assert_eq!(
            devices,
            [PrinterTarget::Bluetooth {
                name: "PT-P300BT6427".to_string(),
                address: "EC:79:49:61:CC:F8".to_string(),
            }]
        );
    }

    #[test]
    fn parses_bluetooth_status_geometry() {
        let response = parse_bluetooth_status("PT-P300BT\t12\tLaminated tape\t128\t180\t64\n")
            .expect("valid status");
        assert!(matches!(
            response,
            PrinterResponse::Connected {
                media_width: 12,
                max_px: 128,
                dpi: 180,
                tape_width_px: 64,
                ..
            }
        ));
    }

    #[test]
    fn rejects_malformed_bluetooth_status() {
        assert!(parse_bluetooth_status("PT-P300BT\t12").is_err());
    }

    #[test]
    fn network_discovery_maps_supported_brother_model_names() {
        assert_eq!(
            model_pid_from_service_text("Brother PT-P900W"),
            Some(0x2085)
        );
        assert_eq!(model_pid_from_service_text("PT-E550W"), Some(0x2060));
        assert_eq!(model_pid_from_service_text("Brother QL-820NWB"), None);
    }

    #[test]
    fn tcp_target_polls_and_prints_without_blocking_the_ui_worker_contract() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut init = [0; 102];
            stream.read_exact(&mut init).unwrap();
            let mut job = Vec::new();
            loop {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                job.push(byte[0]);
                if byte[0] == 0x1A {
                    break;
                }
            }
            stream.write_all(&status_packet(1)).unwrap();
            let mut close_probe = [0; 1];
            let _ = stream.read(&mut close_probe);
            job
        });

        let target = PrinterTarget::Tcp {
            address,
            pid: 0x2060,
        };
        let (tx, rx) = mpsc::channel();
        let raster_lines = [vec![0xAA; 70]];
        let cancellation = ptouch_core::CancellationToken::default();
        assert!(do_print(
            PrintJob {
                target: &target,
                raster_lines: &raster_lines,
                chain_print: false,
                auto_cut: false,
                quality: PrintQuality::Standard,
                tape_width_mm: 12,
                cancellation: &cancellation,
            },
            (&tx, &egui::Context::default()),
        ));
        assert!(matches!(
            rx.recv().unwrap().response,
            PrinterResponse::PrintDone
        ));
        let job = server.join().unwrap();
        assert!(job.starts_with(&[0x1B, 0x69, 0x61, 0x01]));
        assert_eq!(job.last(), Some(&0x1A));
    }

    #[test]
    fn tcp_poll_reports_target_quality_capabilities() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut close_probe = [0; 1];
            let _ = stream.read(&mut close_probe);
        });

        let response = poll_printer(&PrinterTarget::Tcp {
            address,
            pid: 0x2060,
        })
        .unwrap();
        assert!(matches!(
            response,
            PrinterResponse::Connected {
                model_name,
                media_status_available: false,
                media_detected: false,
                quality_modes: false,
                draft_quality_modes: false,
                ..
            } if model_name == "PT-E550W"
        ));
        server.join().unwrap();
    }

    fn status_packet(status_type: u8) -> [u8; 32] {
        let mut packet = [0; 32];
        packet[0] = 0x80;
        packet[1] = 0x20;
        packet[18] = status_type;
        packet
    }
}
