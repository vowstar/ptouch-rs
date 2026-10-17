use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use ptset::{device, settings, usb};

/// Configure Brother PT-P900-series label printers over USB.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Select a printer by USB serial number (default: first Brother printer).
    #[arg(long, global = true)]
    serial: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List connected USB printer interfaces.
    List,
    /// Query printer status (read-only).
    Status {
        /// Also print the raw 32-byte reply.
        #[arg(long)]
        raw: bool,
    },
    /// Read device settings (read-only). With no key, reads all known settings.
    Get {
        key: Option<String>,
        /// Also print the raw reply bytes.
        #[arg(long)]
        raw: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::List => {
            for path in usb::list_interfaces()? {
                println!("{path}");
            }
        }
        Command::Status { raw } => {
            let printer = usb::Printer::open(cli.serial.as_deref())?;
            let status = device::read_status(&printer)?;
            println!("{status}");
            if raw {
                println!("Raw:        {:02X?}", status.raw);
            }
        }
        Command::Get { key, raw } => {
            let selected: Vec<_> = match key.as_deref() {
                Some(k) => vec![settings::find(k).with_context(|| {
                    let keys: Vec<_> = settings::SETTINGS.iter().map(|s| s.key).collect();
                    format!("unknown setting {k:?}; known: {}", keys.join(", "))
                })?],
                None => settings::SETTINGS.iter().collect(),
            };
            let printer = usb::Printer::open(cli.serial.as_deref())?;
            device::initialize(&printer)?;
            for s in selected {
                match device::read_setting(&printer, s) {
                    Ok(r) => {
                        println!("{:<26} {}", s.label, s.describe(r.value));
                        if raw {
                            println!("{:<26} {:02X?}", "", r.reply);
                        }
                    }
                    Err(e) => println!("{:<26} error: {e}", s.label),
                }
            }
        }
    }
    Ok(())
}
