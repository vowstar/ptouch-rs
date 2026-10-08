// SPDX-License-Identifier: GPL-3.0-or-later
use std::io::{self, Read, Write};

pub(super) const MAX_DATA: usize = 65536;
pub(super) const OPEN: u32 = 1;
pub(super) const WRITE: u32 = 2;
pub(super) const READ: u32 = 3;
pub(super) const CLOSE: u32 = 4;
pub(super) const PING: u32 = 5;
pub(super) const OK: u32 = 0;
pub(super) const TIMEOUT: u32 = 1;
pub(super) const ERROR: u32 = 2;

pub(super) struct Frame {
    pub code: u32,
    pub millis: u32,
    pub data: Vec<u8>,
}
impl Frame {
    pub fn write(&self, out: &mut impl Write) -> io::Result<()> {
        if self.data.len() > MAX_DATA {
            return Err(io::Error::other("IPC frame too large"));
        }
        out.write_all(&self.code.to_le_bytes())?;
        out.write_all(&self.millis.to_le_bytes())?;
        out.write_all(&(self.data.len() as u32).to_le_bytes())?;
        out.write_all(&self.data)?;
        out.flush()
    }
    pub fn read(input: &mut impl Read) -> io::Result<Self> {
        let mut header = [0u8; 12];
        input.read_exact(&mut header)?;
        let code = u32::from_le_bytes(header[..4].try_into().unwrap());
        let millis = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let len = u32::from_le_bytes(header[8..].try_into().unwrap()) as usize;
        if len > MAX_DATA {
            return Err(io::Error::other("IPC frame too large"));
        }
        let mut data = vec![0; len];
        input.read_exact(&mut data)?;
        Ok(Self { code, millis, data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_oversized_and_truncated_frames() {
        let mut bytes = Vec::new();
        Frame {
            code: READ,
            millis: 10,
            data: vec![1, 2],
        }
        .write(&mut bytes)
        .unwrap();
        assert_eq!(Frame::read(&mut bytes.as_slice()).unwrap().data, [1, 2]);
        assert!(Frame::read(&mut &bytes[..bytes.len() - 1]).is_err());
        bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Frame::read(&mut bytes.as_slice()).is_err());
    }
}
