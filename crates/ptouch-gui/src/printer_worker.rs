// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Background worker thread for non-blocking printer operations.
//!
//! Owns all USB access and communicates with the UI via mpsc channels.
//! When idle, polls for a connected printer every few seconds.

use std::sync::mpsc;
use std::time::Duration;

use log::{error, info};

use ptouch_core::transport::PtouchDevice;

use crate::state::{PrinterCommand, PrinterResponse};

/// Poll interval: how long the worker waits for a command before polling.
const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// Run the printer worker loop.
///
/// Blocks on `cmd_rx` with a timeout. On timeout, polls for a printer.
/// On receiving a command, executes it. Sends responses via `resp_tx`.
/// Calls `ctx.request_repaint()` after each response so the UI picks it up.
pub fn printer_worker(
    cmd_rx: mpsc::Receiver<PrinterCommand>,
    resp_tx: mpsc::Sender<PrinterResponse>,
    ctx: egui::Context,
) {
    info!("Printer worker started");

    loop {
        match cmd_rx.recv_timeout(POLL_INTERVAL) {
            Ok(PrinterCommand::Poll) => {
                do_poll(&resp_tx, &ctx);
            }
            Ok(PrinterCommand::Print {
                raster_lines,
                chain_print,
                auto_cut,
            }) => {
                do_print(&resp_tx, &ctx, &raster_lines, chain_print, auto_cut);
            }
            Ok(PrinterCommand::FeedAndCut) => {
                do_feed_and_cut(&resp_tx, &ctx);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Idle timeout: poll for printer
                do_poll(&resp_tx, &ctx);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                info!("Printer worker: command channel closed");
                break;
            }
        }
    }
}

/// Poll for a connected printer (query status only, no init).
fn do_poll(resp_tx: &mpsc::Sender<PrinterResponse>, ctx: &egui::Context) {
    let resp = match PtouchDevice::open_first() {
        Ok(mut dev) => {
            let max_px = dev.max_px();
            let model_name = dev.device_info().name.to_string();
            match dev.query_status() {
                Ok(status) => {
                    let media_width = status.media_width;
                    let media_type = status.media_type_name().to_string();
                    let _ = dev.close();
                    PrinterResponse::Connected {
                        model_name,
                        media_width,
                        media_type,
                        max_px,
                    }
                }
                Err(e) => {
                    error!("Poll status error: {}", e);
                    let _ = dev.close();
                    PrinterResponse::Disconnected
                }
            }
        }
        Err(_) => PrinterResponse::Disconnected,
    };
    let _ = resp_tx.send(resp);
    ctx.request_repaint();
}

/// Print raster data to the printer.
fn do_print(
    resp_tx: &mpsc::Sender<PrinterResponse>,
    ctx: &egui::Context,
    raster_lines: &[Vec<u8>],
    chain_print: bool,
    auto_cut: bool,
) {
    let result = (|| -> Result<(), String> {
        let mut dev = PtouchDevice::open_first().map_err(|e| format!("Connect error: {}", e))?;
        if let Err(e) = dev.init() {
            let msg = format!("Init error: {}", e);
            let _ = dev.close();
            return Err(msg);
        }
        let r = dev
            .print_raster(raster_lines, chain_print, auto_cut)
            .map_err(|e| format!("Print error: {}", e));
        let _ = dev.close();
        r
    })();

    let resp = match result {
        Ok(()) => {
            info!("Print successful");
            PrinterResponse::PrintDone
        }
        Err(msg) => {
            error!("{}", msg);
            PrinterResponse::Error(msg)
        }
    };
    let _ = resp_tx.send(resp);
    ctx.request_repaint();
}

/// Feed tape forward and cut.
fn do_feed_and_cut(resp_tx: &mpsc::Sender<PrinterResponse>, ctx: &egui::Context) {
    let result = (|| -> Result<(), String> {
        let mut dev = PtouchDevice::open_first().map_err(|e| format!("Connect error: {}", e))?;
        if let Err(e) = dev.init() {
            let msg = format!("Init error: {}", e);
            let _ = dev.close();
            return Err(msg);
        }
        let r = dev
            .feed_and_cut()
            .map_err(|e| format!("Feed & cut error: {}", e));
        let _ = dev.close();
        r
    })();

    let resp = match result {
        Ok(()) => {
            info!("Feed and cut successful");
            PrinterResponse::FeedAndCutDone
        }
        Err(msg) => {
            error!("{}", msg);
            PrinterResponse::Error(msg)
        }
    };
    let _ = resp_tx.send(resp);
    ctx.request_repaint();
}
