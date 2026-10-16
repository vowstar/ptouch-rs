//! Device settings recovered from brdvset.exe (Printer Setting Tool 1.6.0142, DeviceSettings\PT900).
//!
//! Each setting is an `ISettingItem` subclass whose vtable slot 11 builds the SET command,
//! slot 12 the GET command, slot 13 returns the reply length and slot 14 decodes the reply.
//! Byte layouts below are copied from those builders; `vtable` records where each came from.

pub enum Reply {
    /// 3-byte reply, raw value at index 2.
    Short,
    /// 32-byte status-format reply, raw value at index 30.
    Status,
}

impl Reply {
    pub fn len(&self) -> usize {
        match self {
            Reply::Short => 3,
            Reply::Status => 32,
        }
    }
    pub fn value(&self, reply: &[u8]) -> Option<u8> {
        match self {
            Reply::Short => reply.get(2).copied(),
            Reply::Status => reply.get(30).copied(),
        }
    }
}

pub struct Setting {
    pub key: &'static str,
    pub label: &'static str,
    /// RTTI class and vtable address in brdvset.exe.
    pub class: &'static str,
    pub vtable: u32,
    pub get: &'static [u8],
    /// SET command prefix; the raw value byte is appended.
    pub set_prefix: &'static [u8],
    pub reply: Reply,
    /// Known raw values, from the decompiled reply/UI mapping or Brother's documentation.
    pub values: &'static [(u8, &'static str)],
}

pub const SETTINGS: &[Setting] = &[
    Setting {
        key: "auto-power-off-ac",
        label: "Auto Power Off (AC/DC)",
        class: "ItemAutoOffAc",
        vtable: 0x5ea1c8,
        get: &[0x1B, 0x69, 0x55, 0x41, 0x01, 0x00],
        set_prefix: &[0x1B, 0x69, 0x55, 0x41, 0x00, 0x00],
        reply: Reply::Short,
        values: &[],
    },
    Setting {
        key: "auto-power-off-li",
        label: "Auto Power Off (Li-ion)",
        class: "ItemAutoOffLi",
        vtable: 0x5ea220,
        get: &[0x1B, 0x69, 0x55, 0x41, 0x01, 0x01],
        set_prefix: &[0x1B, 0x69, 0x55, 0x41, 0x00, 0x01],
        reply: Reply::Short,
        values: &[],
    },
    Setting {
        key: "auto-power-on",
        label: "Auto Power On",
        class: "ItemAutoOn",
        vtable: 0x5ea274,
        get: &[0x1B, 0x69, 0x55, 0x70, 0x01],
        set_prefix: &[0x1B, 0x69, 0x55, 0x70, 0x00],
        reply: Reply::Status,
        values: &[],
    },
    Setting {
        key: "command-mode",
        label: "Command Mode",
        class: "ItemCommandMode",
        vtable: 0x5ea670,
        get: &[0x1B, 0x69, 0x55, 0x61, 0x01],
        set_prefix: &[0x1B, 0x69, 0x55, 0x61, 0x00],
        reply: Reply::Status,
        // Matches Brother's documented ESC i a dynamic command modes.
        values: &[(0x00, "ESC/P"), (0x01, "Raster"), (0x03, "P-touch Template")],
    },
    Setting {
        key: "save-print-log",
        label: "Save Printer Print Log",
        class: "ItemSavePrintLog",
        vtable: 0x5eb388,
        get: &[0x1B, 0x69, 0x58, 0x67, 0x31, 0x02, 0x00, 0x00, 0x00],
        set_prefix: &[0x1B, 0x69, 0x58, 0x67, 0x32, 0x03, 0x00, 0x00, 0x00],
        reply: Reply::Short,
        values: &[],
    },
];

pub fn find(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.key == key)
}

impl Setting {
    pub fn describe(&self, raw: u8) -> String {
        match self.values.iter().find(|(v, _)| *v == raw) {
            Some((_, name)) => format!("{name} (0x{raw:02X})"),
            None => format!("0x{raw:02X}"),
        }
    }
    /// Only values confirmed from decompiled code or Brother documentation may be written.
    pub fn is_known_value(&self, raw: u8) -> bool {
        self.values.iter().any(|(v, _)| *v == raw)
    }
    pub fn writable(&self) -> bool {
        !self.values.is_empty()
    }
    pub fn set_command(&self, raw: u8) -> Vec<u8> {
        let mut cmd = self.set_prefix.to_vec();
        cmd.push(raw);
        cmd
    }
}
