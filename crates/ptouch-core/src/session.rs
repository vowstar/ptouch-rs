// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>
// SPDX-FileCopyrightText: Dominic Radermacher and the ptouch-print contributors
//
// Portions derived from ptouch-print, licensed GPL-3.0-or-later:
// https://git.familie-radermacher.ch/linux/ptouch-print.git

//! Transport-independent printer state and command lifecycle.

use crate::CancellationToken;
use crate::model::{Dialect, ModelProfile};
use crate::p300bt;
use crate::{
    device::DeviceFlags,
    error::{PtouchError, Result},
    protocol,
    status::{PrinterStatus, STATUS_PACKET_SIZE},
};
use log::{debug, info, warn};
use std::time::{Duration, Instant};

/// Internal byte transport. Native handles need not be Send or Sync.
pub(crate) trait Transport {
    fn send(&self, data: &[u8], timeout: Duration) -> Result<()>;
    fn receive(&self, buf: &mut [u8], timeout: Duration) -> Result<usize>;
    fn close(self) -> Result<()>;
}

/// Default timeout for byte transfers.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(5);

/// PT-18RZ pauses USB reception while feeding an internal buffer. A five-second
/// write deadline truncates long jobs; measured pauses fit within 30 seconds.
/// The USB transport polls cancellation while waiting for buffer space.
const PRINT_TRANSFER_TIMEOUT: Duration = Duration::from_secs(30);

/// Short timeout for flushing stale input.
const FLUSH_TIMEOUT: Duration = Duration::from_millis(100);

/// Delay between status read retries.
const STATUS_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Maximum number of status read retries.
const STATUS_MAX_RETRIES: usize = 10;

/// Maximum time to wait without hearing anything from the printer after a
/// print command. The deadline restarts on every status transfer, so a label
/// that takes minutes to feed is bounded by printer silence, not by the total
/// length of the job.
const PRINT_STATUS_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Bound each USB read so transient silence cannot consume the whole idle
/// deadline in a single transfer.
const PRINT_STATUS_POLL_TIMEOUT: Duration = Duration::from_millis(250);

/// Never hand libusb a zero timeout, which means "wait forever".
const PRINT_STATUS_MIN_POLL_TIMEOUT: Duration = Duration::from_millis(1);

/// Pace the loop after a zero-length bulk transfer so a printer that keeps
/// completing empty reads cannot spin a core until the deadline expires.
const ZERO_LENGTH_TRANSFER_DELAY: Duration = Duration::from_millis(10);

pub(crate) struct PrinterSession<T: Transport> {
    pub(crate) transport: T,
    profile: ModelProfile,
    status_frames: StatusFrameBuffer,
    status: Option<PrinterStatus>,
    tape_width_px: Option<u16>,
    media_width_override: Option<u8>,
    media_type_override: Option<u8>,
    text_color_override: Option<u8>,
    initialized: bool,
    pub(crate) cancellation: CancellationToken,
    pub(crate) job_timeout: Duration,
}

impl<T: Transport> PrinterSession<T> {
    pub(crate) fn new(transport: T, profile: ModelProfile) -> Self {
        Self {
            transport,
            profile,
            status_frames: StatusFrameBuffer::new(),
            status: None,
            tape_width_px: None,
            media_width_override: None,
            media_type_override: None,
            text_color_override: None,
            initialized: false,
            cancellation: CancellationToken::default(),
            job_timeout: Duration::from_secs(600),
        }
    }
    /// Get the device flags.
    pub fn flags(&self) -> DeviceFlags {
        self.profile.flags
    }

    /// Get the most recently read printer status, if available.
    pub fn status(&self) -> Option<&PrinterStatus> {
        self.status.as_ref()
    }

    /// Get the tape width in pixels, if known.
    pub fn tape_width_px(&self) -> Option<u16> {
        self.tape_width_px
    }

    fn media_width(&self) -> Option<u8> {
        self.media_width_override
            .or_else(|| self.status.as_ref().map(|status| status.media_width))
            .filter(|width| *width > 0)
    }

    fn requires_manual_media_width(&self) -> bool {
        self.profile.dialect == Dialect::Tcp
    }

    pub(crate) fn set_media_width_override(&mut self, width_mm: u8) -> Result<()> {
        if self.profile.raster_width_px <= 128 && (width_mm == 21 || width_mm > 24) {
            return Err(PtouchError::UnknownTapeWidth(width_mm));
        }
        self.tape_width_px = self.profile.tape_width_px(width_mm);
        if self.tape_width_px.is_none() {
            return Err(PtouchError::UnknownTapeWidth(width_mm));
        }
        self.media_width_override = Some(width_mm);
        Ok(())
    }

    pub(crate) fn update_network_status(
        &mut self,
        status: PrinterStatus,
    ) -> Result<&PrinterStatus> {
        self.tape_width_px = self.profile.tape_width_px(status.media_width);
        self.status = Some(status);
        self.status.as_ref().ok_or(PtouchError::NotInitialized)
    }

    #[cfg(test)]
    pub(crate) fn set_media_details_for_test(&mut self, media_type: u8, text_color: u8) {
        self.media_type_override = Some(media_type);
        self.text_color_override = Some(text_color);
    }

    fn advanced_quality_matches_media(&self) -> bool {
        let media_type = self
            .status
            .as_ref()
            .map(|status| status.media_type)
            .or(self.media_type_override);
        let text_color = self
            .status
            .as_ref()
            .map(|status| status.text_color)
            .or(self.text_color_override);
        media_type == Some(0x01) && text_color == Some(0x08)
    }

    /// Get the raster transfer width, which may exceed the printable area.
    pub fn raster_width_px(&self) -> u16 {
        self.profile.raster_width_px
    }

    /// Whether the device has been initialized.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Send raw bytes through the selected transport.
    pub fn send(&self, data: &[u8]) -> Result<()> {
        self.cancellation.check()?;
        self.transport.send(data, TRANSFER_TIMEOUT)
    }

    pub fn receive(&self, buf: &mut [u8]) -> Result<usize> {
        self.receive_with_timeout(buf, TRANSFER_TIMEOUT)
    }

    fn receive_with_timeout(&self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        self.cancellation.check()?;
        self.transport.receive(buf, timeout)
    }

    fn flush_input(&mut self) -> Result<()> {
        self.status_frames.clear();
        drain_input(
            |buf, timeout| {
                self.cancellation.check()?;
                self.transport.receive(buf, timeout)
            },
            FLUSH_TIMEOUT,
            4096,
        )
    }

    /// Initialize the printer.
    ///
    /// Sends the init sequence (100 zeros + ESC @) and queries the status.
    /// Raster start is sent per-job in `print_raster()`.
    pub fn init(&mut self) -> Result<()> {
        self.initialized = false;
        match self.profile.dialect {
            Dialect::Tcp => {
                self.send(&protocol::cmd_init())?;
                self.status = None;
                self.tape_width_px = None;
                self.media_width_override = None;
                self.media_type_override = None;
                self.text_color_override = None;
            }
            Dialect::Usb => {
                self.flush_input()?;
                self.send(&protocol::cmd_init())?;
                print_status_is_ready(self.get_status()?)?;
            }
            Dialect::P300Bt => {
                self.flush_input()?;
                self.send(&p300bt::cmd_init())?;
                print_status_is_ready(self.get_status()?)?;
            }
        }

        if self
            .profile
            .flags
            .contains(DeviceFlags::AUTO_STATUS_NOTIFICATION)
        {
            // Enable the phase changes used by the post-print readiness handshake.
            self.send(&protocol::cmd_auto_status_notification(true))?;
        }

        self.initialized = true;
        info!(
            "Device initialized: {}, tape={}mm ({}px)",
            self.profile.name,
            self.status.as_ref().map_or(0, |s| s.media_width),
            self.tape_width_px.unwrap_or(0)
        );

        Ok(())
    }

    /// Query printer status without sending the init command.
    ///
    /// Flushes stale USB data and reads the printer status. Unlike
    /// [`init`](Self::init), this does not send the 100-zero + ESC @
    /// reset sequence, so it will not disturb the printer.
    pub fn query_status(&mut self) -> Result<&PrinterStatus> {
        if self.profile.dialect == Dialect::Tcp {
            return Err(PtouchError::UnsupportedOperation(
                "Status requests are unavailable for this transport/model",
            ));
        }
        self.flush_input()?;
        self.get_status()
    }

    /// Request and read the printer status.
    ///
    /// Sends the status request command and reads the 32-byte response.
    /// Retries up to STATUS_MAX_RETRIES times with STATUS_RETRY_DELAY
    /// between attempts.
    /// Updates internal status and tape width fields.
    pub fn get_status(&mut self) -> Result<&PrinterStatus> {
        if self.profile.dialect == Dialect::P300Bt {
            return self.get_p300bt_status();
        }
        if self.profile.dialect == Dialect::Tcp {
            return Err(PtouchError::UnsupportedOperation(
                "Status requests are unavailable for this transport/model",
            ));
        }
        self.send(&protocol::cmd_status_request())?;

        let mut buf = [0u8; STATUS_PACKET_SIZE];
        let mut frames = StatusFrameBuffer::new();
        let mut response = None;

        // Retry loop: sleep then read
        for attempt in 0..STATUS_MAX_RETRIES {
            std::thread::sleep(STATUS_RETRY_DELAY);

            match self.receive_with_timeout(&mut buf, TRANSFER_TIMEOUT) {
                Ok(0) => {
                    debug!("Empty status read (attempt {})", attempt + 1);
                    continue;
                }
                Ok(n) if n > buf.len() => {
                    return Err(PtouchError::StatusError(
                        "Transport returned more bytes than the receive buffer".into(),
                    ));
                }
                Ok(n) => {
                    frames.push(&buf[..n]);
                    while let Some(packet) = frames.pop() {
                        // A late phase/completion notification from an earlier
                        // page is not a reply to this status request.
                        let status = match parse_status_packet(&packet, "Invalid status header") {
                            Ok(status) => status,
                            Err(error) => {
                                self.flush_input()?;
                                return Err(error);
                            }
                        };
                        if matches!(status.status_type, 0 | 2 | 4) || status.has_error() {
                            response = Some(status);
                            break;
                        }
                        debug!(
                            "Ignoring {} while awaiting a status reply",
                            status.status_type_name()
                        );
                    }
                }
                Err(PtouchError::Timeout) => {
                    debug!("Status read timeout (attempt {})", attempt + 1);
                    continue;
                }
                Err(e) => return Err(e),
            }

            if response.is_some() {
                break;
            }
            debug!(
                "Short status read ({} bytes, attempt {})",
                frames.len(),
                attempt + 1
            );
        }

        let Some(status) = response else {
            // Flush junk data before returning error
            self.flush_input()?;
            return Err(PtouchError::StatusError(format!(
                "Status packet too short: {} bytes (expected {})",
                frames.len(),
                STATUS_PACKET_SIZE
            )));
        };

        debug!(
            "Status: type={}, media_width={}mm, media_type={}, tape_color={}, text_color={}",
            status.status_type_name(),
            status.media_width,
            status.media_type_name(),
            status.tape_color_name(),
            status.text_color_name()
        );

        if status.has_error() {
            warn!("Printer reports error: {}", status.error_description());
        }

        // Resolve tape width to pixel count for this printer's resolution,
        // clamped to the head width (wide tapes exceed narrow heads).
        self.tape_width_px = self.profile.tape_width_px(status.media_width);
        if self.tape_width_px.is_none() && status.media_width != 0 {
            warn!("Unknown tape width: {} mm", status.media_width);
        }

        self.status = Some(status);

        // The unwrap is safe because we just assigned Some above
        Ok(self.status.as_ref().unwrap())
    }

    /// Print raster image data.
    ///
    /// `lines` is a slice of raster line buffers, each `ceil(max_px/8)` bytes
    /// wide. The printer will print one raster line per entry.
    ///
    /// # Arguments
    /// * `lines` - Raster image data, one byte-slice per line.
    /// * `chain_print` - If true, don't cut the tape (chain mode).
    /// * `precut` - If true AND device supports precut, send precut command.
    /// * `quality` - Print quality mode (device must support non-standard).
    ///
    /// # Errors
    ///
    /// Returns [`PtouchError::NotInitialized`] if [`init`](Self::init) was
    /// not called, or [`PtouchError::UnsupportedQuality`] if a non-standard
    /// quality is requested on a device without quality modes.
    pub fn print_raster(
        &mut self,
        lines: &[Vec<u8>],
        chain_print: bool,
        precut: bool,
        quality: protocol::PrintQuality,
    ) -> Result<()> {
        if !self.initialized {
            return Err(PtouchError::NotInitialized);
        }

        if let Some(status) = &self.status {
            print_status_is_ready(status)?;
        }

        let quality_supported = match quality {
            protocol::PrintQuality::Standard => true,
            protocol::PrintQuality::HighRes => {
                self.profile
                    .flags
                    .intersects(DeviceFlags::LEGACY_HIRES | DeviceFlags::ADVANCED_HIRES)
                    && (!self.profile.flags.contains(DeviceFlags::ADVANCED_HIRES)
                        || self.advanced_quality_matches_media())
            }
            protocol::PrintQuality::Draft => self.profile.flags.contains(DeviceFlags::LEGACY_HIRES),
        };
        if !quality_supported {
            return Err(PtouchError::UnsupportedQuality(
                self.profile.name.to_string(),
            ));
        }

        let media_width = self
            .media_width()
            .or_else(|| (!self.requires_manual_media_width()).then_some(0))
            .ok_or(PtouchError::UnknownTapeWidth(0))?;

        if self.profile.dialect == Dialect::P300Bt {
            let status = self.status.as_ref().ok_or(PtouchError::NotInitialized)?;
            let job = p300bt::build_print_job(lines, status, chain_print)?;
            let result = self
                .send_job(job)
                .and_then(|_| self.receive_p300bt_completion());
            if result.is_err() {
                self.initialized = false;
            }
            return result;
        }

        let opts = protocol::JobOptions {
            media_width,
            margin_dots: ((u32::from(self.profile.dpi) * 10 + 63) / 127) as u16,
            chain_print,
            precut,
            quality,
        };

        let job = protocol::build_print_job(lines, self.profile.flags, &opts);
        info!(
            "Print job: model={}, tape={}mm, input_lines={}, length={:.2}mm, dpi={}, quality={:?}, wire_bytes={}, final={:02x?}",
            self.profile.name,
            opts.media_width,
            lines.len(),
            lines.len() as f64 * 25.4 / f64::from(self.profile.dpi),
            self.profile.dpi,
            quality,
            job.iter().map(Vec::len).sum::<usize>(),
            job.last().unwrap_or(&Vec::new())
        );
        let started = Instant::now();
        if let Err(error) = self.send_job(job) {
            self.initialized = false;
            return Err(error);
        }

        let result = if self
            .profile
            .flags
            .contains(DeviceFlags::WAIT_FOR_RECEIVE_READY)
        {
            self.wait_until_ready(started)
        } else {
            self.receive_print_completion(started)
        };
        if result.is_err() {
            self.initialized = false;
        }
        result
    }

    /// Feed tape forward and cut.
    ///
    /// Prints a minimal blank strip (a few blank raster lines) then
    /// ejects and cuts. The printer needs actual raster data to engage
    /// the feed mechanism.
    pub fn feed_and_cut(&mut self) -> Result<()> {
        if self.profile.dialect == Dialect::P300Bt {
            return Err(PtouchError::UnsupportedOperation(
                "PT-P300BT has a manual cutter",
            ));
        }
        if !self.initialized {
            return Err(PtouchError::NotInitialized);
        }

        let media_width = self
            .media_width()
            .or_else(|| (!self.requires_manual_media_width()).then_some(0))
            .ok_or(PtouchError::UnknownTapeWidth(0))?;

        if let Some(status) = &self.status {
            print_status_is_ready(status)?;
        }

        // One blank line makes the printer engage the feed mechanism.
        let lines = vec![protocol::rasterline_blank(self.profile.raster_width_px)];
        let opts = protocol::JobOptions {
            media_width,
            ..protocol::JobOptions::default()
        };

        let job = protocol::build_print_job(&lines, self.profile.flags, &opts);
        let started = Instant::now();
        if let Err(error) = self.send_job(job) {
            self.initialized = false;
            return Err(error);
        }

        if self
            .profile
            .flags
            .contains(DeviceFlags::WAIT_FOR_RECEIVE_READY)
        {
            if let Err(error) = self.wait_until_ready(started) {
                self.initialized = false;
                return Err(error);
            }
        } else if let Err(error) = self.receive_print_completion(started) {
            self.initialized = false;
            return Err(error);
        }
        info!("Feed and cut");
        Ok(())
    }

    fn send_job(&self, job: Vec<Vec<u8>>) -> Result<()> {
        let start = Instant::now();
        let total_chunks = job.len();
        let mut sent_bytes = 0usize;
        let mut sent_lines = 0usize;
        for (index, chunk) in job.into_iter().enumerate() {
            self.cancellation.check()?;
            let remaining = self
                .job_timeout
                .checked_sub(start.elapsed())
                .filter(|duration| *duration >= PRINT_STATUS_MIN_POLL_TIMEOUT)
                .ok_or(PtouchError::Timeout)?;
            let transfer_timeout = match self.profile.dialect {
                Dialect::Usb | Dialect::Tcp => PRINT_TRANSFER_TIMEOUT,
                Dialect::P300Bt => TRANSFER_TIMEOUT,
            };
            let command_started = Instant::now();
            if let Err(error) = self.transport.send(&chunk, remaining.min(transfer_timeout)) {
                warn!(
                    "Print transfer stopped at command {}/{}, after {} raster lines and {} bytes: {}",
                    index + 1,
                    total_chunks,
                    sent_lines,
                    sent_bytes,
                    error
                );
                return Err(error);
            }
            if command_started.elapsed() >= Duration::from_secs(1) {
                debug!(
                    "Command {} resumed after {:.3}s of USB flow control",
                    index + 1,
                    command_started.elapsed().as_secs_f64()
                );
            }
            sent_bytes += chunk.len();
            if chunk.first() == Some(&0x47) || chunk == [0x5a] {
                sent_lines += 1;
                if sent_lines.is_multiple_of(1024) {
                    debug!("Sent {} raster lines, {} bytes", sent_lines, sent_bytes);
                }
            }
        }
        debug!(
            "Sent all {} commands, {} raster lines, {} bytes in {:.3}s; awaiting device status",
            total_chunks,
            sent_lines,
            sent_bytes,
            start.elapsed().as_secs_f64()
        );
        Ok(())
    }

    fn wait_until_ready(&mut self, started: Instant) -> Result<()> {
        let remaining = self
            .job_timeout
            .checked_sub(started.elapsed())
            .unwrap_or_default();
        let result = receive_print_status_bounded(
            |buf, timeout| self.receive_with_timeout(buf, timeout),
            PRINT_STATUS_IDLE_TIMEOUT,
            remaining,
        );
        match result {
            Ok(status) => {
                self.status = Some(status);
                Ok(())
            }
            Err(PtouchError::Timeout) => Err(PtouchError::CompletionUnknown),
            Err(error) => Err(error),
        }
    }

    /// Models without a readiness handshake still need a completion notification.
    fn receive_print_completion(&mut self, job_started: Instant) -> Result<()> {
        let started = Instant::now();
        let completion_timeout = self
            .job_timeout
            .checked_sub(job_started.elapsed())
            .unwrap_or_default();
        let mut frames = StatusFrameBuffer::new();
        loop {
            let remaining = completion_timeout
                .checked_sub(started.elapsed())
                .filter(|duration| *duration >= PRINT_STATUS_MIN_POLL_TIMEOUT)
                .ok_or(PtouchError::CompletionUnknown)?;
            let mut response = [0u8; STATUS_PACKET_SIZE];
            match self.receive_with_timeout(&mut response, remaining.min(PRINT_STATUS_POLL_TIMEOUT))
            {
                Ok(n) if n > response.len() => {
                    return Err(PtouchError::StatusError("Invalid transport length".into()));
                }
                Ok(0) => std::thread::sleep(ZERO_LENGTH_TRANSFER_DELAY),
                Ok(n) => {
                    frames.push(&response[..n]);
                    if let Some(packet) = frames.pop() {
                        let status = parse_status_packet(&packet, "Invalid completion header")?;
                        // Streaming models can report a receiving phase between
                        // internal buffers, before the final raster data prints.
                        // Only a completion notification confirms this page.
                        print_status_is_ready(&status)?;
                        if status.status_type == 1 {
                            self.status = Some(status);
                            return Ok(());
                        }
                    }
                }
                Err(PtouchError::Timeout) => {}
                Err(error) => return Err(error),
            }
        }
    }

    fn get_p300bt_status(&mut self) -> Result<&PrinterStatus> {
        self.send(&protocol::cmd_status_request())?;
        let transport = &self.transport;
        let cancellation = &self.cancellation;
        let status = read_p300bt_status(
            &mut self.status_frames,
            |buf, timeout| {
                cancellation.check()?;
                transport.receive(buf, timeout)
            },
            Duration::from_secs(10),
            false,
        )?;
        self.tape_width_px = self.profile.tape_width_px(status.media_width);
        self.status = Some(status);
        Ok(self.status.as_ref().unwrap())
    }

    fn receive_p300bt_completion(&mut self) -> Result<()> {
        // A fixed total deadline; phase changes alone never complete the job.
        let transport = &self.transport;
        let cancellation = &self.cancellation;
        self.status = Some(read_p300bt_status(
            &mut self.status_frames,
            |buf, timeout| {
                cancellation.check()?;
                transport.receive(buf, timeout)
            },
            Duration::from_secs(60),
            true,
        )?);
        Ok(())
    }

    pub(crate) fn close(self) -> Result<()> {
        self.transport.close()?;
        info!("Device closed: {}", self.profile.name);
        Ok(())
    }
}

fn read_p300bt_status<F>(
    frames: &mut StatusFrameBuffer,
    mut receive: F,
    timeout: Duration,
    wait_for_completion: bool,
) -> Result<PrinterStatus>
where
    F: FnMut(&mut [u8], Duration) -> Result<usize>,
{
    let start = Instant::now();
    loop {
        while let Some(packet) = frames.pop() {
            let status = parse_status_packet(&packet, "Invalid PT-P300BT status header")?;
            if status.brother_code != 0x42
                || status.series_code != 0x30
                || status.model_code != 0x72
            {
                return Err(PtouchError::StatusError(
                    "Unexpected PT-P300BT status identity".into(),
                ));
            }
            // Error and power-off checks are shared with the USB readiness loop,
            // but P300BT completion does not require a later receiving phase.
            print_status_is_ready(&status)?;
            if (!wait_for_completion && status.status_type == 0)
                || (wait_for_completion && status.status_type == 1)
            {
                return Ok(status);
            }
        }
        let remaining = timeout
            .checked_sub(start.elapsed())
            .filter(|d| !d.is_zero())
            .ok_or(PtouchError::Timeout)?;
        // A larger receive buffer deliberately permits coalesced notifications.
        let mut bytes = [0u8; STATUS_PACKET_SIZE * 4];
        match receive(&mut bytes, remaining.min(PRINT_STATUS_POLL_TIMEOUT)) {
            Ok(n) if n > bytes.len() => {
                return Err(PtouchError::StatusError(
                    "Transport returned more bytes than the receive buffer".into(),
                ));
            }
            Ok(0) => std::thread::sleep(ZERO_LENGTH_TRANSFER_DELAY),
            Ok(n) => frames.push(&bytes[..n]),
            Err(PtouchError::Timeout) => {}
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
fn receive_print_status<F>(receive: F) -> Result<PrinterStatus>
where
    F: FnMut(&mut [u8], Duration) -> Result<usize>,
{
    receive_print_status_bounded(receive, PRINT_STATUS_IDLE_TIMEOUT, Duration::from_secs(600))
}

/// Bounded drain shared by USB and Bluetooth. Non-timeout errors are preserved.
fn drain_input<F>(mut receive: F, timeout: Duration, byte_limit: usize) -> Result<()>
where
    F: FnMut(&mut [u8], Duration) -> Result<usize>,
{
    let started = Instant::now();
    let mut total = 0usize;
    let mut bytes = [0u8; 64];
    loop {
        let remaining = timeout
            .checked_sub(started.elapsed())
            .filter(|duration| *duration >= PRINT_STATUS_MIN_POLL_TIMEOUT)
            .ok_or(PtouchError::InputNotIdle)?;
        match receive(&mut bytes, remaining) {
            Ok(0) | Err(PtouchError::Timeout) => return Ok(()),
            Ok(n) if n > bytes.len() => {
                return Err(PtouchError::StatusError(
                    "Transport returned more bytes than the receive buffer".into(),
                ));
            }
            Ok(n) => {
                total += n;
                if total >= byte_limit {
                    return Err(PtouchError::InputNotIdle);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
fn receive_print_status_with_timeout<F>(receive: F, idle_timeout: Duration) -> Result<PrinterStatus>
where
    F: FnMut(&mut [u8], Duration) -> Result<usize>,
{
    receive_print_status_bounded(receive, idle_timeout, Duration::from_secs(600))
}

fn receive_print_status_bounded<F>(
    mut receive: F,
    idle_timeout: Duration,
    total_timeout: Duration,
) -> Result<PrinterStatus>
where
    F: FnMut(&mut [u8], Duration) -> Result<usize>,
{
    let started = Instant::now();
    let mut last_transfer = Instant::now();
    let mut frames = StatusFrameBuffer::new();

    loop {
        let Some(remaining) = idle_timeout.checked_sub(last_transfer.elapsed()) else {
            return Err(PtouchError::Timeout);
        };

        let total_remaining = total_timeout
            .checked_sub(started.elapsed())
            .filter(|duration| *duration >= PRINT_STATUS_MIN_POLL_TIMEOUT)
            .ok_or(PtouchError::Timeout)?;
        let remaining = remaining.min(total_remaining);
        let mut transfer = [0u8; STATUS_PACKET_SIZE];
        let read_timeout = remaining
            .min(PRINT_STATUS_POLL_TIMEOUT)
            .max(PRINT_STATUS_MIN_POLL_TIMEOUT);
        let read = match receive(&mut transfer, read_timeout) {
            Ok(read) => read,
            Err(PtouchError::Timeout) => {
                debug!("No print status available yet");
                continue;
            }
            Err(error) => return Err(error),
        };

        if read > transfer.len() {
            return Err(PtouchError::StatusError(format!(
                "USB read reported {} bytes for a {}-byte buffer",
                read,
                transfer.len()
            )));
        }

        // A successful zero-byte bulk transfer is USB framing, not a Brother
        // status packet. Keep waiting within the overall deadline.
        if read == 0 {
            debug!("Ignoring zero-length USB transfer after print");
            std::thread::sleep(ZERO_LENGTH_TRANSFER_DELAY);
            continue;
        }

        // Real bytes mean the job is still alive. Restart the idle deadline so
        // a label that feeds for a long time is not cut short mid-print.
        last_transfer = Instant::now();

        frames.push(&transfer[..read]);
        if frames.len() < STATUS_PACKET_SIZE {
            debug!(
                "Accumulated {} of {} print-status bytes",
                frames.len(),
                STATUS_PACKET_SIZE
            );
            continue;
        }

        let Some(response) = frames.pop() else {
            continue;
        };
        let status = parse_status_packet(&response, "Invalid status header after print")?;

        debug!(
            "Print status: type={}, phase_type={:#04x}, phase={:#04x}{:02x}",
            status.status_type_name(),
            status.phase_type,
            status.phase_number_hi,
            status.phase_number_lo
        );

        if print_status_is_ready(&status)? {
            debug!("Printer is ready to receive the next page");
            return Ok(status);
        }
    }
}

struct StatusFrameBuffer {
    pending: Vec<u8>,
}

impl StatusFrameBuffer {
    fn new() -> Self {
        Self {
            pending: Vec::with_capacity(STATUS_PACKET_SIZE * 2),
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
    }

    fn clear(&mut self) {
        self.pending.clear();
    }

    fn len(&self) -> usize {
        self.pending.len()
    }

    fn pop(&mut self) -> Option<[u8; STATUS_PACKET_SIZE]> {
        if self.pending.len() < STATUS_PACKET_SIZE {
            return None;
        }

        let mut frame = [0u8; STATUS_PACKET_SIZE];
        frame.copy_from_slice(&self.pending[..STATUS_PACKET_SIZE]);
        self.pending.drain(..STATUS_PACKET_SIZE);
        Some(frame)
    }
}

fn parse_status_packet(
    response: &[u8; STATUS_PACKET_SIZE],
    invalid_header_message: &str,
) -> Result<PrinterStatus> {
    debug!("Raw printer status: {:02x?}", response);
    let status = PrinterStatus::from_bytes(response)
        .ok_or_else(|| PtouchError::StatusError("Failed to parse status packet".to_string()))?;

    if status.print_head_mark != 0x80 || status.size != 0x20 {
        return Err(PtouchError::StatusError(format!(
            "{}: mark={:#04x} size={:#04x}",
            invalid_header_message, status.print_head_mark, status.size
        )));
    }

    Ok(status)
}

fn print_status_is_ready(status: &PrinterStatus) -> Result<bool> {
    if status.has_error() || status.status_type == 0x02 {
        let description = if status.has_error() {
            status.error_description()
        } else {
            "Printer reported an unspecified error".to_string()
        };
        return Err(PtouchError::StatusError(description));
    }

    if status.status_type == 0x04 {
        return Err(PtouchError::StatusError("Printer turned off".to_string()));
    }

    Ok(status.is_waiting_to_receive())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    #[test]
    fn drain_rejects_continuous_input_and_preserves_transport_errors() {
        let mut reads = 0;
        let result = drain_input(
            |buf, timeout| {
                assert!(!timeout.is_zero());
                reads += 1;
                buf.fill(0xff);
                Ok(buf.len())
            },
            Duration::from_secs(1),
            128,
        );
        assert!(matches!(result, Err(PtouchError::InputNotIdle)));
        assert_eq!(reads, 2);
        assert!(matches!(
            drain_input(
                |_, _| Err(PtouchError::UsbError(rusb::Error::NoDevice)),
                Duration::from_secs(1),
                128
            ),
            Err(PtouchError::UsbError(rusb::Error::NoDevice))
        ));
        assert!(drain_input(|buf, _| Ok(buf.len() + 1), Duration::from_secs(1), 128).is_err());
    }

    #[test]
    fn repeated_printing_status_cannot_extend_total_job_deadline() {
        let packet = status_packet(6, 1);
        let mut reads = 0;
        let result = receive_print_status_bounded(
            |buf, _| {
                reads += 1;
                if reads > 1000 {
                    return Err(PtouchError::UnsupportedOperation("test watchdog"));
                }
                std::thread::sleep(Duration::from_millis(1));
                buf.copy_from_slice(&packet);
                Ok(packet.len())
            },
            Duration::from_secs(1),
            Duration::from_millis(10),
        );
        assert!(matches!(result, Err(PtouchError::Timeout)));
    }

    #[test]
    fn cancellation_stops_initialization_before_any_write() {
        let mut session = usb_session(DeviceFlags::NONE);
        session.cancellation.cancel();
        assert!(matches!(session.init(), Err(PtouchError::Cancelled)));
        assert!(session.transport.writes.borrow().is_empty());
    }

    #[test]
    fn target_standard_jobs_match_usb_session_golden_bytes() {
        for (pid, raster_width, packbits) in [(0x2060, 16, true), (0x2085, 70, false)] {
            let info = crate::device::find_device(crate::device::BROTHER_VENDOR_ID, pid)
                .expect("target model exists");
            let mut session = PrinterSession::new(ScriptedTransport::new(), info.into());
            session.init().unwrap();
            session.transport.writes.borrow_mut().clear();
            session
                .transport
                .reads
                .borrow_mut()
                .push_back(status_packet(1, 1).to_vec());

            session
                .print_raster(
                    &[vec![0xAA; raster_width]],
                    false,
                    false,
                    protocol::PrintQuality::Standard,
                )
                .unwrap();

            let mut raster = if packbits {
                vec![0x47, 0x11, 0x00, 0x0F]
            } else {
                vec![0x47, 0x46, 0x00]
            };
            raster.extend(vec![0xAA; raster_width]);
            let mut expected = vec![
                vec![0x1B, 0x69, 0x61, 0x01],
                vec![0x1B, 0x69, 0x7A, 0x00, 0x00, 12, 0x00, 1, 0, 0, 0, 0, 0],
            ];
            if packbits {
                expected.push(vec![0x4D, 0x02]);
            }
            expected.extend([raster, vec![0x1A]]);
            assert_eq!(*session.transport.writes.borrow(), expected);
        }
    }

    #[test]
    fn target_profiles_reject_unverified_draft_quality_before_writing() {
        for pid in [0x2060, 0x2085] {
            let info = crate::device::find_device(crate::device::BROTHER_VENDOR_ID, pid)
                .expect("target model exists");
            let mut session = PrinterSession::new(ScriptedTransport::new(), info.into());
            session.initialized = true;
            assert!(matches!(
                session.print_raster(
                    &[vec![0xFF; usize::from(info.max_px).div_ceil(8)]],
                    false,
                    false,
                    protocol::PrintQuality::Draft,
                ),
                Err(PtouchError::UnsupportedQuality(_))
            ));
            assert!(session.transport.writes.borrow().is_empty());
        }
    }

    #[test]
    fn advanced_high_resolution_rejects_media_without_black_text_before_writing() {
        let info = crate::device::find_device(crate::device::BROTHER_VENDOR_ID, 0x2085)
            .expect("target model exists");
        let mut session = PrinterSession::new(ScriptedTransport::new(), info.into());
        session.initialized = true;
        session.set_media_width_override(24).unwrap();
        session.set_media_details_for_test(0x01, 0x07);

        assert!(matches!(
            session.print_raster(
                &[vec![0xFF; usize::from(info.max_px).div_ceil(8)]],
                false,
                false,
                protocol::PrintQuality::HighRes,
            ),
            Err(PtouchError::UnsupportedQuality(_))
        ));
        assert!(session.transport.writes.borrow().is_empty());
    }

    #[test]
    fn unconfirmed_readiness_stops_later_pages() {
        let mut session = usb_session(DeviceFlags::WAIT_FOR_RECEIVE_READY);
        session.init().unwrap();
        session.job_timeout = Duration::from_millis(5);
        let result = session.print_raster(
            &[vec![0; 16]],
            true,
            false,
            protocol::PrintQuality::Standard,
        );
        assert!(matches!(result, Err(PtouchError::CompletionUnknown)));
        let writes = session.transport.writes.borrow().len();
        assert!(matches!(
            session.print_raster(
                &[vec![0; 16]],
                false,
                false,
                protocol::PrintQuality::Standard
            ),
            Err(PtouchError::NotInitialized)
        ));
        assert_eq!(session.transport.writes.borrow().len(), writes);
    }

    fn status_packet(status_type: u8, phase_type: u8) -> [u8; STATUS_PACKET_SIZE] {
        let mut packet = [0u8; STATUS_PACKET_SIZE];
        packet[0] = 0x80;
        packet[1] = 0x20;
        packet[18] = status_type;
        packet[19] = phase_type;
        packet
    }

    struct ScriptedTransport {
        writes: std::cell::RefCell<Vec<Vec<u8>>>,
        fail_on_write: std::cell::Cell<Option<usize>>,
        reads: std::cell::RefCell<VecDeque<Vec<u8>>>,
        first_read_delay: std::cell::Cell<Duration>,
        write_timeouts: std::cell::RefCell<Vec<Duration>>,
        status_reply: std::cell::Cell<Option<[u8; STATUS_PACKET_SIZE]>>,
        // Prove the session works with a transport that cannot cross threads.
        _owner: std::rc::Rc<()>,
    }
    impl ScriptedTransport {
        fn new() -> Self {
            Self {
                writes: Default::default(),
                fail_on_write: Default::default(),
                reads: Default::default(),
                first_read_delay: Default::default(),
                write_timeouts: Default::default(),
                status_reply: Default::default(),
                _owner: std::rc::Rc::new(()),
            }
        }
    }
    impl Transport for ScriptedTransport {
        fn send(&self, data: &[u8], timeout: Duration) -> Result<()> {
            self.writes.borrow_mut().push(data.to_vec());
            self.write_timeouts.borrow_mut().push(timeout);
            if self.fail_on_write.get() == Some(self.writes.borrow().len()) {
                return Err(PtouchError::SendFailed("Injected transfer failure".into()));
            }
            if data == [0x1b, 0x69, 0x53] {
                let mut packet = status_packet(0, 0);
                packet[2] = 0x42;
                packet[3] = 0x30;
                packet[4] = 0x72;
                packet[10] = 12;
                let packet = self.status_reply.take().unwrap_or(packet);
                self.reads
                    .borrow_mut()
                    .extend([packet[..11].to_vec(), packet[11..].to_vec()]);
            }
            Ok(())
        }
        fn receive(&self, buf: &mut [u8], _timeout: Duration) -> Result<usize> {
            std::thread::sleep(self.first_read_delay.replace(Duration::ZERO));
            let bytes = self
                .reads
                .borrow_mut()
                .pop_front()
                .ok_or(PtouchError::Timeout)?;
            buf[..bytes.len()].copy_from_slice(&bytes);
            Ok(bytes.len())
        }
        fn close(self) -> Result<()> {
            Ok(())
        }
    }
    fn usb_session(flags: DeviceFlags) -> PrinterSession<ScriptedTransport> {
        PrinterSession::new(
            ScriptedTransport::new(),
            ModelProfile {
                dialect: Dialect::Usb,
                name: "Test USB",
                raster_width_px: 128,
                dpi: 180,
                flags,
            },
        )
    }
    fn p300bt_packet(kind: u8, phase: u8) -> [u8; STATUS_PACKET_SIZE] {
        let mut packet = status_packet(kind, phase);
        packet[2] = 0x42;
        packet[3] = 0x30;
        packet[4] = 0x72;
        packet[10] = 12;
        packet
    }
    #[test]
    fn p300bt_profile_keeps_printable_and_raster_widths_separate() {
        let mut session = PrinterSession::new(ScriptedTransport::new(), ModelProfile::P300BT);
        session.init().unwrap();
        assert_eq!(session.raster_width_px(), 128);
        assert_eq!(session.tape_width_px(), Some(64));
        assert_eq!(ModelProfile::P300BT.tape_width_px(9), None);
        let mut reset = vec![0; 64];
        reset.extend([0x1b, 0x40, 0x1b, 0x69, 0x61, 1]);
        assert_eq!(
            *session.transport.writes.borrow(),
            vec![reset, vec![0x1b, 0x69, 0x53]]
        );
        assert!(matches!(
            session.feed_and_cut(),
            Err(PtouchError::UnsupportedOperation(_))
        ));
    }
    #[test]
    fn p300bt_failed_job_stops_sending_and_cannot_be_replayed() {
        let mut session = PrinterSession::new(ScriptedTransport::new(), ModelProfile::P300BT);
        session.init().unwrap();
        session.transport.writes.borrow_mut().clear();
        session.transport.fail_on_write.set(Some(2));
        assert!(matches!(
            session.print_raster(
                &[vec![0; 16]],
                false,
                false,
                protocol::PrintQuality::Standard
            ),
            Err(PtouchError::SendFailed(_))
        ));
        assert_eq!(session.transport.writes.borrow().len(), 2);
        assert!(!session.is_initialized());
        assert!(matches!(
            session.print_raster(
                &[vec![0; 16]],
                false,
                false,
                protocol::PrintQuality::Standard
            ),
            Err(PtouchError::NotInitialized)
        ));
        assert_eq!(session.transport.writes.borrow().len(), 2);
    }

    #[test]
    fn p300bt_completion_handles_fragmented_and_coalesced_notifications() {
        let printing = p300bt_packet(6, 1);
        let done = p300bt_packet(1, 1);
        let mut transfers = VecDeque::from([
            printing[..7].to_vec(),
            [printing[7..].as_ref(), done.as_ref()].concat(),
        ]);
        let result = read_p300bt_status(
            &mut StatusFrameBuffer::new(),
            |buf, _| {
                let bytes = transfers.pop_front().ok_or(PtouchError::Timeout)?;
                buf[..bytes.len()].copy_from_slice(&bytes);
                Ok(bytes.len())
            },
            Duration::from_secs(1),
            true,
        )
        .unwrap();
        assert_eq!(result.status_type, 1);
        assert_eq!(result.phase_type, 1); // No later receiving phase is required.
        assert!(transfers.is_empty());
    }
    #[test]
    fn p300bt_errors_override_completion_and_disconnects_propagate() {
        let mut done = p300bt_packet(1, 1);
        done[8] = 8;
        let result = read_p300bt_status(
            &mut StatusFrameBuffer::new(),
            |buf, _| {
                buf[..32].copy_from_slice(&done);
                Ok(32)
            },
            Duration::from_secs(1),
            true,
        );
        assert!(matches!(result, Err(PtouchError::StatusError(_))));
        let off = p300bt_packet(4, 0);
        assert!(
            read_p300bt_status(
                &mut StatusFrameBuffer::new(),
                |buf, _| {
                    buf[..32].copy_from_slice(&off);
                    Ok(32)
                },
                Duration::from_secs(1),
                true
            )
            .is_err()
        );
        assert!(matches!(
            read_p300bt_status(
                &mut StatusFrameBuffer::new(),
                |_, _| Err(PtouchError::Bluetooth("Disconnected".into())),
                Duration::from_secs(1),
                true
            ),
            Err(PtouchError::Bluetooth(_))
        ));
    }
    #[test]
    fn p300bt_query_skips_stale_notifications_and_checks_identity() {
        let mut transfers = VecDeque::from([p300bt_packet(1, 1), p300bt_packet(0, 0)]);
        let status = read_p300bt_status(
            &mut StatusFrameBuffer::new(),
            |buf, _| {
                let bytes = transfers.pop_front().ok_or(PtouchError::Timeout)?;
                buf[..32].copy_from_slice(&bytes);
                Ok(32)
            },
            Duration::from_secs(1),
            false,
        )
        .unwrap();
        assert_eq!(status.status_type, 0);
        let mut wrong = p300bt_packet(0, 0);
        wrong[4] = 0;
        assert!(
            read_p300bt_status(
                &mut StatusFrameBuffer::new(),
                |buf, _| {
                    buf[..32].copy_from_slice(&wrong);
                    Ok(32)
                },
                Duration::from_secs(1),
                false
            )
            .is_err()
        );
    }
    #[test]
    fn p300bt_phase_changes_do_not_extend_the_completion_deadline() {
        let packet = p300bt_packet(6, 1);
        let result = read_p300bt_status(
            &mut StatusFrameBuffer::new(),
            |buf, _| {
                buf[..32].copy_from_slice(&packet);
                Ok(32)
            },
            Duration::from_millis(2),
            true,
        );
        assert!(matches!(result, Err(PtouchError::Timeout)));
        assert!(matches!(
            read_p300bt_status(
                &mut StatusFrameBuffer::new(),
                |_, _| Ok(0),
                Duration::ZERO,
                true
            ),
            Err(PtouchError::Timeout)
        ));
    }

    #[test]
    fn usb_session_keeps_initialization_sequence_and_tape_resolution() {
        let mut session = usb_session(DeviceFlags::AUTO_STATUS_NOTIFICATION);
        session.init().unwrap();
        let mut reset = vec![0; 100];
        reset.extend([0x1b, 0x40]);
        assert_eq!(
            *session.transport.writes.borrow(),
            vec![reset, vec![0x1b, 0x69, 0x53], vec![0x1b, 0x69, 0x21, 0]]
        );
        assert_eq!(session.tape_width_px(), Some(76));
        assert!(session.is_initialized());
    }
    #[test]
    fn usb_session_keeps_plain_job_and_reports_unknown_completion() {
        let mut session = usb_session(DeviceFlags::NONE);
        session.init().unwrap();
        session.transport.writes.borrow_mut().clear();
        // Missing confirmation must not be reported as a completed label.
        session.job_timeout = Duration::from_millis(5);
        let result = session.print_raster(
            &[vec![0; 16], vec![0x80; 16]],
            false,
            false,
            protocol::PrintQuality::Standard,
        );
        assert!(matches!(result, Err(PtouchError::CompletionUnknown)));
        assert!(!session.is_initialized());
        let mut raster = vec![0x47, 16, 0];
        raster.extend([0x80; 16]);
        assert_eq!(
            *session.transport.writes.borrow(),
            vec![vec![0x1b, 0x69, 0x52, 1], vec![0x5a], raster, vec![0x1a]]
        );
    }

    #[test]
    fn usb_completion_uses_job_deadline_instead_of_five_second_transfer_timeout() {
        let mut session = usb_session(DeviceFlags::NONE);
        session.initialized = true;
        session.job_timeout = Duration::from_secs(8);
        // The first notification arrives after the old five-second deadline.
        // A printing phase is followed by the actual completion notification.
        session
            .transport
            .first_read_delay
            .set(Duration::from_millis(5100));
        session
            .transport
            .reads
            .borrow_mut()
            .extend([status_packet(6, 1).to_vec(), status_packet(1, 1).to_vec()]);
        session
            .print_raster(
                &[vec![0; 16]],
                false,
                false,
                protocol::PrintQuality::Standard,
            )
            .unwrap();
        assert_eq!(session.status().unwrap().status_type, 1);
        assert!(session.is_initialized());
    }

    #[test]
    fn usb_receiving_phase_does_not_hide_a_later_print_error() {
        let mut session = usb_session(DeviceFlags::NONE);
        session.initialized = true;
        let mut error = status_packet(2, 1);
        error[9] = 4;
        session
            .transport
            .reads
            .borrow_mut()
            .extend([status_packet(6, 0).to_vec(), error.to_vec()]);
        assert!(matches!(
            session.print_raster(&[vec![0; 16]], false, false, protocol::PrintQuality::Standard),
            Err(PtouchError::StatusError(message)) if message == "Communication error"
        ));
        assert!(!session.is_initialized());
    }

    #[test]
    fn usb_status_request_ignores_late_phase_notifications() {
        let mut session = usb_session(DeviceFlags::NONE);
        session
            .transport
            .reads
            .borrow_mut()
            .push_back(status_packet(6, 1).to_vec());
        assert_eq!(session.get_status().unwrap().status_type, 0);
        assert!(session.transport.reads.borrow().is_empty());
    }

    #[test]
    fn usb_initialization_rejects_device_errors_but_retains_diagnostics() {
        for (kind, error_bits, message) in [
            (0, 4, "Communication error"),
            (2, 0, "Printer reported an unspecified error"),
            (4, 0, "Printer turned off"),
        ] {
            let mut session = usb_session(DeviceFlags::AUTO_STATUS_NOTIFICATION);
            let mut packet = status_packet(kind, 0);
            packet[9] = error_bits;
            session.transport.status_reply.set(Some(packet));
            assert!(
                matches!(session.init(), Err(PtouchError::StatusError(actual)) if actual == message)
            );
            assert!(!session.is_initialized());
            assert_eq!(session.status().unwrap().status_type, kind);
            assert_eq!(session.status().unwrap().error_info_2, error_bits);
            assert_eq!(
                *session.transport.writes.borrow(),
                [protocol::cmd_init(), protocol::cmd_status_request()]
            );
        }
    }

    #[test]
    fn usb_print_and_feed_reject_errors_observed_after_initialization() {
        for (kind, error_bits) in [(0, 4), (2, 0), (4, 0)] {
            let mut session = usb_session(DeviceFlags::NONE);
            session.init().unwrap();
            let mut packet = status_packet(kind, 0);
            packet[9] = error_bits;
            session.transport.status_reply.set(Some(packet));
            // Queries still return the device's diagnostic information.
            assert_eq!(session.query_status().unwrap().status_type, kind);
            session.transport.writes.borrow_mut().clear();
            assert!(matches!(
                session.print_raster(
                    &[vec![0; 16]],
                    false,
                    false,
                    protocol::PrintQuality::Standard
                ),
                Err(PtouchError::StatusError(_))
            ));
            assert!(matches!(
                session.feed_and_cut(),
                Err(PtouchError::StatusError(_))
            ));
            assert!(session.transport.writes.borrow().is_empty());
        }
    }

    #[test]
    fn usb_print_transfer_budget_allows_backpressure_and_respects_job_deadline() {
        for seconds in [2, 60] {
            let mut session = usb_session(DeviceFlags::NONE);
            session.initialized = true;
            session.job_timeout = Duration::from_secs(seconds);
            session
                .transport
                .reads
                .borrow_mut()
                .push_back(status_packet(1, 1).to_vec());
            session
                .print_raster(
                    &[vec![0; 16]],
                    false,
                    false,
                    protocol::PrintQuality::Standard,
                )
                .unwrap();
            for timeout in session.transport.write_timeouts.borrow().iter() {
                assert!(*timeout >= Duration::from_millis(1));
                assert!(*timeout <= Duration::from_secs(seconds.min(30)));
                if seconds == 60 {
                    assert_eq!(*timeout, Duration::from_secs(30));
                }
            }
        }
    }
    #[test]
    fn usb_session_query_does_not_reset_or_enable_notifications() {
        for (kind, error_bits) in [(0, 0), (0, 4), (2, 0), (4, 0)] {
            let mut session = usb_session(DeviceFlags::AUTO_STATUS_NOTIFICATION);
            let mut packet = status_packet(kind, 0);
            packet[9] = error_bits;
            session.transport.status_reply.set(Some(packet));
            let status = session.query_status().unwrap();
            assert_eq!(status.status_type, kind);
            assert_eq!(status.error_info_2, error_bits);
            assert_eq!(
                *session.transport.writes.borrow(),
                vec![vec![0x1b, 0x69, 0x53]]
            );
            assert!(!session.is_initialized());
        }
    }

    #[test]
    fn print_status_waits_until_reception_is_possible() {
        let mut packets = VecDeque::from([
            status_packet(0x06, 0x01),
            status_packet(0x01, 0x00),
            status_packet(0x06, 0x00),
        ]);

        let status = receive_print_status(|buf, _timeout| {
            let packet = packets.pop_front().ok_or(PtouchError::Timeout)?;
            buf.copy_from_slice(&packet);
            Ok(packet.len())
        })
        .unwrap();

        assert_eq!(status.status_type, 0x06);
        assert_eq!(status.phase_type, 0x00);
        assert!(packets.is_empty());
    }

    #[test]
    fn print_status_does_not_accept_printing_completed_as_ready() {
        let mut packets = VecDeque::from([status_packet(0x01, 0x00)]);

        let result = receive_print_status_with_timeout(
            |buf, _timeout| {
                let packet = packets.pop_front().ok_or(PtouchError::Timeout)?;
                buf.copy_from_slice(&packet);
                Ok(packet.len())
            },
            Duration::from_millis(1),
        );

        assert!(matches!(result, Err(PtouchError::Timeout)));
    }

    #[test]
    fn print_status_propagates_printer_errors() {
        let mut packet = status_packet(0x02, 0x00);
        packet[8] = 0x04;

        let result = receive_print_status(|buf, _timeout| {
            buf.copy_from_slice(&packet);
            Ok(packet.len())
        });

        assert!(matches!(
            result,
            Err(PtouchError::StatusError(message)) if message == "Cutter jam"
        ));
    }

    #[test]
    fn print_status_propagates_power_off() {
        let packet = status_packet(0x04, 0x00);

        let result = receive_print_status(|buf, _timeout| {
            buf.copy_from_slice(&packet);
            Ok(packet.len())
        });

        assert!(matches!(
            result,
            Err(PtouchError::StatusError(message)) if message == "Printer turned off"
        ));
    }

    #[test]
    fn print_status_times_out_after_incomplete_packet() {
        let mut transfers = VecDeque::from([Ok(vec![0u8; 12]), Err(PtouchError::Timeout)]);

        let result = receive_print_status_with_timeout(
            |buf, _timeout| match transfers.pop_front().unwrap_or(Err(PtouchError::Timeout)) {
                Ok(transfer) => {
                    buf[..transfer.len()].copy_from_slice(&transfer);
                    Ok(transfer.len())
                }
                Err(error) => Err(error),
            },
            Duration::from_millis(1),
        );

        assert!(matches!(result, Err(PtouchError::Timeout)));
    }

    #[test]
    fn print_status_ignores_zero_length_usb_transfers() {
        let mut transfers = VecDeque::from([
            Vec::new(),
            status_packet(0x06, 0x01).to_vec(),
            status_packet(0x01, 0x00).to_vec(),
            status_packet(0x06, 0x00).to_vec(),
        ]);

        let status = receive_print_status(|buf, _timeout| {
            let transfer = transfers.pop_front().ok_or(PtouchError::Timeout)?;
            buf[..transfer.len()].copy_from_slice(&transfer);
            Ok(transfer.len())
        })
        .unwrap();

        assert!(status.is_waiting_to_receive());
        assert!(transfers.is_empty());
    }

    #[test]
    fn print_status_accumulates_fragmented_packets() {
        let ready = status_packet(0x06, 0x00);
        let mut transfers = VecDeque::from([ready[..11].to_vec(), ready[11..].to_vec()]);

        let status = receive_print_status(|buf, _timeout| {
            let transfer = transfers.pop_front().ok_or(PtouchError::Timeout)?;
            buf[..transfer.len()].copy_from_slice(&transfer);
            Ok(transfer.len())
        })
        .unwrap();

        assert!(status.is_waiting_to_receive());
        assert!(transfers.is_empty());
    }

    #[test]
    fn status_frame_buffer_preserves_partial_next_packet() {
        let first = status_packet(0x01, 0x00);
        let second = status_packet(0x06, 0x00);
        let mut frames = StatusFrameBuffer::new();

        frames.push(&first[..9]);
        frames.push(&[first[9..].as_ref(), second[..7].as_ref()].concat());

        assert_eq!(frames.pop(), Some(first));
        assert_eq!(frames.len(), 7);

        frames.push(&second[7..]);
        assert_eq!(frames.pop(), Some(second));
        assert_eq!(frames.len(), 0);
    }

    #[test]
    fn print_status_tolerates_transient_usb_timeouts() {
        let ready = status_packet(0x06, 0x00);
        let mut transfers = VecDeque::from([
            Err(PtouchError::Timeout),
            Err(PtouchError::Timeout),
            Ok(ready.to_vec()),
        ]);

        let status = receive_print_status(|buf, _timeout| {
            match transfers.pop_front().ok_or(PtouchError::Timeout)? {
                Ok(transfer) => {
                    buf[..transfer.len()].copy_from_slice(&transfer);
                    Ok(transfer.len())
                }
                Err(error) => Err(error),
            }
        })
        .unwrap();

        assert!(status.is_waiting_to_receive());
        assert!(transfers.is_empty());
    }

    #[test]
    fn consecutive_pages_each_wait_for_their_receiving_phase() {
        let mut transfers = VecDeque::from([
            status_packet(0x06, 0x01),
            status_packet(0x01, 0x00),
            status_packet(0x06, 0x00),
            status_packet(0x06, 0x01),
            status_packet(0x01, 0x00),
            status_packet(0x06, 0x00),
        ]);
        let mut receive = |buf: &mut [u8], _timeout: Duration| {
            let packet = transfers.pop_front().ok_or(PtouchError::Timeout)?;
            buf.copy_from_slice(&packet);
            Ok(packet.len())
        };

        let first = receive_print_status(&mut receive).unwrap();
        let second = receive_print_status(&mut receive).unwrap();

        assert!(first.is_waiting_to_receive());
        assert!(second.is_waiting_to_receive());
        assert!(transfers.is_empty());
    }

    #[test]
    fn print_status_has_an_overall_deadline() {
        let result = receive_print_status_with_timeout(|_, _| Ok(0), Duration::ZERO);

        assert!(matches!(result, Err(PtouchError::Timeout)));
    }

    #[test]
    fn print_status_deadline_restarts_on_every_transfer() {
        let mut packets = VecDeque::from([
            status_packet(0x06, 0x01),
            status_packet(0x06, 0x01),
            status_packet(0x01, 0x00),
            status_packet(0x06, 0x00),
        ]);

        // Every gap stays inside the idle deadline while the total elapsed
        // time runs past it, which is what a long label looks like.
        let status = receive_print_status_with_timeout(
            |buf, _timeout| {
                std::thread::sleep(Duration::from_millis(100));
                let packet = packets.pop_front().ok_or(PtouchError::Timeout)?;
                buf.copy_from_slice(&packet);
                Ok(packet.len())
            },
            Duration::from_millis(300),
        )
        .unwrap();

        assert!(status.is_waiting_to_receive());
        assert!(packets.is_empty());
    }

    #[test]
    fn print_status_paces_zero_length_transfers() {
        let mut transfers = 0usize;

        let result = receive_print_status_with_timeout(
            |_, _| {
                transfers += 1;
                Ok(0)
            },
            Duration::from_millis(50),
        );

        assert!(matches!(result, Err(PtouchError::Timeout)));
        assert!(
            transfers <= 50,
            "zero-length transfers were not paced ({} reads)",
            transfers
        );
    }

    #[test]
    fn p300bt_status_stream_preserves_error_notifications_across_calls() {
        fn packet(kind: u8, error: u8) -> Vec<u8> {
            let mut bytes = p300bt_packet(kind, 0).to_vec();
            bytes[8] = error;
            bytes
        }

        for limit in [7, 32, 64] {
            let mut stream: VecDeque<u8> =
                [packet(0, 0), packet(2, 8), packet(1, 0)].concat().into();
            let mut receive = |buf: &mut [u8], _: Duration| {
                if stream.is_empty() {
                    return Err(PtouchError::Timeout);
                }
                let count = stream.len().min(buf.len()).min(limit);
                for byte in &mut buf[..count] {
                    *byte = stream.pop_front().unwrap();
                }
                Ok(count)
            };
            let mut frames = StatusFrameBuffer::new();

            let result =
                read_p300bt_status(&mut frames, &mut receive, Duration::from_secs(1), false)
                    .and_then(|_| {
                        read_p300bt_status(&mut frames, &mut receive, Duration::from_secs(1), true)
                    })
                    .map(|_| ());

            assert!(
                matches!(result, Err(PtouchError::StatusError(ref message)) if message == "Weak battery"),
                "lost error at read limit {limit}: {result:?}"
            );
        }
    }

    fn p300bt_session_after_print(notification_prefix: usize) -> PrinterSession<ScriptedTransport> {
        let mut session = PrinterSession::new(ScriptedTransport::new(), ModelProfile::P300BT);
        session.init().unwrap();

        let completed = p300bt_packet(1, 1);
        let receiving = p300bt_packet(6, 0);
        // A read can include completion and the start of the next notification.
        // Its remaining bytes stay in the transport until the next operation.
        session
            .transport
            .reads
            .borrow_mut()
            .push_back([completed.as_ref(), &receiving[..notification_prefix]].concat());
        if notification_prefix < receiving.len() {
            session
                .transport
                .reads
                .borrow_mut()
                .push_back(receiving[notification_prefix..].to_vec());
        }
        session
            .print_raster(
                &[vec![0; 16]],
                false,
                false,
                protocol::PrintQuality::Standard,
            )
            .unwrap();
        session
    }

    #[test]
    fn p300bt_query_after_print_flushes_partial_notifications() {
        for prefix in 0..=STATUS_PACKET_SIZE {
            let mut session = p300bt_session_after_print(prefix);
            let status = session.query_status().unwrap_or_else(|error| {
                panic!("query failed with {prefix} buffered bytes: {error}")
            });
            assert_eq!(status.status_type, 0);
            assert_eq!(status.media_width, 12);
            assert!(!status.has_error());
            assert_eq!(session.tape_width_px(), Some(64));
        }
    }

    #[test]
    fn p300bt_init_after_print_flushes_partial_notifications() {
        for prefix in 0..=STATUS_PACKET_SIZE {
            let mut session = p300bt_session_after_print(prefix);
            session.init().unwrap_or_else(|error| {
                panic!("init failed with {prefix} buffered bytes: {error}")
            });
            let status = session.status().unwrap();
            assert_eq!(status.status_type, 0);
            assert_eq!(status.media_width, 12);
            assert!(!status.has_error());
            assert_eq!(session.tape_width_px(), Some(64));
            assert!(session.is_initialized());
        }
    }
}
