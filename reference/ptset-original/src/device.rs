//! High-level printer operations shared by the CLI and GUI.

use crate::settings::Setting;
use crate::status::{self, Status};
use crate::usb::Printer;
use anyhow::{Result, bail};
use std::time::Duration;

const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

/// Clear any partial command and reset the printer's command parser (ESC @).
pub fn initialize(printer: &Printer) -> Result<()> {
    printer.write(&[0u8; 100])?;
    printer.write(&[0x1B, 0x40])
}

pub fn read_status(printer: &Printer) -> Result<Status> {
    printer.write(&status::request())?;
    let mut buf = [0u8; 64];
    let n = printer.read(&mut buf, REPLY_TIMEOUT)?;
    Status::parse(&buf[..n])
}

pub struct Reading {
    pub value: u8,
    pub reply: Vec<u8>,
}

/// Send a setting's GET command and decode the reply.
pub fn read_setting(printer: &Printer, setting: &Setting) -> Result<Reading> {
    printer.write(setting.get)?;
    let mut buf = [0u8; 64];
    let n = printer.read(&mut buf, REPLY_TIMEOUT)?;
    let reply = buf[..n].to_vec();
    match (n == setting.reply.len(), setting.reply.value(&reply)) {
        (true, Some(value)) => Ok(Reading { value, reply }),
        _ => bail!("unexpected {n}-byte reply {:02X?}", reply),
    }
}

/// Write a setting, then read it back and confirm the printer stored the value.
pub fn write_setting(printer: &Printer, setting: &Setting, raw: u8) -> Result<Reading> {
    if !setting.is_known_value(raw) {
        bail!("refusing to write unverified value 0x{raw:02X} to {}", setting.label);
    }
    printer.write(&setting.set_command(raw))?;
    let reading = read_setting(printer, setting)?;
    if reading.value != raw {
        bail!(
            "{}: wrote 0x{raw:02X} but printer reports 0x{:02X}",
            setting.label,
            reading.value
        );
    }
    Ok(reading)
}
