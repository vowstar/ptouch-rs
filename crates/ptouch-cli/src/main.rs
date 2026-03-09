// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Command-line tool for Brother P-Touch label printers.
//!
//! Supports printing text labels, PNG images, or combinations of both.
//! Can also export labels to PNG files for preview.

use std::path::Path;
use std::process;

use clap::{Parser, Subcommand, ValueEnum};
use log::debug;

use ptouch_core::device::{self, DeviceFlags, DeviceInfo};
use ptouch_core::error::PtouchError;
use ptouch_core::tape;
use ptouch_core::transport::PtouchDevice;

use ptouch_render::bitmap::LabelBitmap;
use ptouch_render::image_loader;
use ptouch_render::raster;
use ptouch_render::text::{TextAlign, TextRenderer};

// ---------------------------------------------------------------------------
// CLI argument definitions
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "ptouch", version, about = "Brother P-Touch label printer tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print labels with text, images, or both
    Print(PrintArgs),
    /// Show printer and tape information
    Info(InfoArgs),
    /// List supported printer models
    List,
    /// Launch GUI mode
    Gui,
}

#[derive(clap::Args)]
struct PrintArgs {
    /// Text lines to print (each argument = one line, max 4)
    #[arg(value_name = "TEXT")]
    text: Vec<String>,

    /// Print a PNG image
    #[arg(short = 'i', long)]
    image: Option<String>,

    /// Export to PNG file instead of printing
    #[arg(short = 'o', long)]
    output: Option<String>,

    /// Font name
    #[arg(short = 'f', long, default_value = "DejaVuSans")]
    font: String,

    /// Font size in points (auto-detected if not set)
    #[arg(short = 's', long)]
    size: Option<f32>,

    /// Font top/bottom margin in pixels
    #[arg(short = 'm', long, default_value = "0")]
    margin: u32,

    /// Text alignment
    #[arg(short = 'a', long, value_enum, default_value = "left")]
    align: AlignArg,

    /// Force tape width in pixels (use with -o for PNG export without printer)
    #[arg(short = 'w', long)]
    tape_width: Option<u32>,

    /// Add a cut mark
    #[arg(short = 'c', long)]
    cut: bool,

    /// Add padding in pixels
    #[arg(short = 'p', long)]
    pad: Option<u32>,

    /// Skip final feed and cut (for chained labels)
    #[arg(long)]
    chain: bool,

    /// Cut before label
    #[arg(long)]
    precut: bool,

    /// Number of copies
    #[arg(long, default_value = "1")]
    copies: u32,

    /// Printer timeout in seconds
    #[arg(long, default_value = "1")]
    timeout: u32,

    /// Enable debug output
    #[arg(long)]
    debug: bool,
}

#[derive(clap::Args)]
struct InfoArgs {
    /// Enable debug output
    #[arg(long)]
    debug: bool,

    /// Printer timeout in seconds
    #[arg(long, default_value = "1")]
    timeout: u32,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum AlignArg {
    Left,
    Center,
    Right,
}

impl AlignArg {
    fn to_text_align(self) -> TextAlign {
        match self {
            AlignArg::Left => TextAlign::Left,
            AlignArg::Center => TextAlign::Center,
            AlignArg::Right => TextAlign::Right,
        }
    }
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::List => execute_list(),
        Commands::Gui => execute_gui(),
        Commands::Info(args) => {
            init_logging(args.debug);
            if let Err(e) = execute_info(&args) {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        }
        Commands::Print(args) => {
            init_logging(args.debug);
            if let Err(e) = execute_print(&args) {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        }
    }
}

/// Initialize env_logger with optional debug level.
fn init_logging(debug: bool) {
    let level = if debug { "debug" } else { "warn" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level))
        .format_timestamp(None)
        .init();
}

// ---------------------------------------------------------------------------
// Subcommand: list
// ---------------------------------------------------------------------------

/// Print a table of all supported printer models.
fn execute_list() {
    let devices = device::supported_devices();
    println!(
        "Supported Brother P-Touch printers ({} models):",
        devices.len()
    );
    println!();
    println!(
        "  {:<30} {:>6} {:>6} {:>4}  Max Pixels",
        "Model", "VID", "PID", "DPI"
    );
    println!("  {}", "-".repeat(70));
    for dev in devices {
        let flags = format_flags(dev);
        println!(
            "  {:<30} 0x{:04x} 0x{:04x} {:>4}  {:>6}  {}",
            dev.name, dev.vid, dev.pid, dev.dpi, dev.max_px, flags
        );
    }
}

/// Format device flags into a human-readable string.
fn format_flags(dev: &DeviceInfo) -> String {
    let mut parts = Vec::new();
    if dev.flags.contains(DeviceFlags::RASTER_PACKBITS) {
        parts.push("packbits");
    }
    if dev.flags.contains(DeviceFlags::HAS_PRECUT) {
        parts.push("precut");
    }
    if dev.flags.contains(DeviceFlags::P700_INIT) {
        parts.push("p700-init");
    }
    if dev.flags.contains(DeviceFlags::USE_INFO_CMD) {
        parts.push("info-cmd");
    }
    if dev.flags.contains(DeviceFlags::PLITE) {
        parts.push("plite");
    }
    if dev.flags.contains(DeviceFlags::UNSUP_RASTER) {
        parts.push("no-raster");
    }
    if dev.flags.contains(DeviceFlags::D460BT_MAGIC) {
        parts.push("d460bt");
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("[{}]", parts.join(", "))
    }
}

// ---------------------------------------------------------------------------
// Subcommand: gui
// ---------------------------------------------------------------------------

/// Print a message directing users to the GUI application.
fn execute_gui() {
    println!("Use ptouch-gui for the graphical interface.");
}

// ---------------------------------------------------------------------------
// Subcommand: info
// ---------------------------------------------------------------------------

/// Open the printer and display status and tape information.
fn execute_info(_args: &InfoArgs) -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = PtouchDevice::open_first()?;
    dev.init()?;

    // Clone the status so we release the mutable borrow on dev.
    let status = dev.get_status()?.clone();

    println!("Printer Information");
    println!("  Model:          {}", dev.device_info().name);
    println!("  Status:         {}", status.status_type_name());
    println!("  Media type:     {}", status.media_type_name());
    println!("  Media width:    {} mm", status.media_width);
    println!("  Tape color:     {}", status.tape_color_name());
    println!("  Text color:     {}", status.text_color_name());

    if status.has_error() {
        println!("  Errors:         {}", status.error_description());
    }

    let tape_width_px = dev.tape_width_px();
    let max_px = dev.max_px();
    let dpi = dev.device_info().dpi;

    println!();
    println!("Tape Details");
    if let Some(px) = tape_width_px {
        println!("  Tape width:     {} px", px);
    } else {
        println!("  Tape width:     unknown");
    }
    println!("  Max printable:  {} px", max_px);
    println!("  Resolution:     {} DPI", dpi);

    // Try to find tape info from the table
    let tapes = tape::supported_tapes();
    for t in tapes {
        if Some(t.pixels) == tape_width_px {
            println!("  Tape size:      {} mm", t.width_mm);
            println!("  Margin:         {:.1} mm", t.margin_mm);
            break;
        }
    }

    dev.close()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Subcommand: print
// ---------------------------------------------------------------------------

/// Build a label from text/images and either print it or save as PNG.
fn execute_print(args: &PrintArgs) -> Result<(), Box<dyn std::error::Error>> {
    // Validate arguments
    if args.text.is_empty() && args.image.is_none() {
        eprintln!("Error: nothing to print (provide text or --image)");
        process::exit(1);
    }

    if args.text.len() > 4 {
        eprintln!("Error: at most 4 text lines are supported");
        process::exit(1);
    }

    if args.tape_width.is_some() && args.output.is_none() {
        eprintln!("Error: --tape-width requires --output");
        process::exit(1);
    }

    // Determine the print width and optionally open the device
    let (print_width, max_px, mut device): (u32, u16, Option<PtouchDevice>) =
        if let Some(w) = args.tape_width {
            // PNG-only mode, no printer needed
            debug!("PNG-only mode with forced tape width: {} px", w);
            (w, w as u16, None)
        } else {
            // Connect to the printer
            debug!("Connecting to printer...");
            let mut dev = PtouchDevice::open_first()?;
            dev.init()?;
            let _status = dev.get_status()?;
            let width = dev.tape_width_px().ok_or_else(|| {
                PtouchError::StatusError("Could not determine tape width".to_string())
            })?;
            let max = dev.max_px();
            debug!("Printer tape width: {} px, max: {} px", width, max);
            (u32::from(width), max, Some(dev))
        };

    // Build the label bitmap
    let bitmap = build_label(args, print_width)?;

    // Output: save to PNG or print to device
    if let Some(ref output_path) = args.output {
        bitmap.save_png(Path::new(output_path))?;
        let tape_mm = bitmap.width() as f64 / 180.0 * 25.4;
        println!(
            "Saved to '{}' ({}x{} px, {:.1} mm of tape)",
            output_path,
            bitmap.width(),
            bitmap.height(),
            tape_mm
        );
    } else if let Some(ref mut dev) = device {
        print_to_device(dev, &bitmap, max_px, args)?;
    } else {
        eprintln!("Error: no output destination (use --output or connect a printer)");
        process::exit(1);
    }

    // Close the device if we opened one
    if let Some(dev) = device {
        dev.close()?;
    }

    Ok(())
}

/// Compose a label bitmap from text, image, cut marks, and padding.
fn build_label(
    args: &PrintArgs,
    print_width: u32,
) -> Result<LabelBitmap, Box<dyn std::error::Error>> {
    let mut result: Option<LabelBitmap> = None;

    // Render text if provided
    if !args.text.is_empty() {
        let mut renderer = TextRenderer::new();
        let lines: Vec<&str> = args.text.iter().map(|s| s.as_str()).collect();
        let align = args.align.to_text_align();

        debug!(
            "Rendering {} text line(s), font={}, size={:?}, margin={}, align={:?}",
            lines.len(),
            args.font,
            args.size,
            args.margin,
            args.align
        );

        let text_bitmap = renderer.render_text(
            &lines,
            print_width,
            &args.font,
            args.size,
            args.margin,
            align,
        )?;

        result = Some(append_bitmap(result, text_bitmap));
    }

    // Load and append image if provided
    if let Some(ref img_path) = args.image {
        debug!("Loading image: {}", img_path);
        let img_bitmap = image_loader::load_png(Path::new(img_path))?;
        result = Some(append_bitmap(result, img_bitmap));
    }

    // Add cut mark if requested
    if args.cut {
        debug!("Adding cut mark");
        let mark = make_cutmark(print_width);
        result = Some(append_bitmap(result, mark));
    }

    // Add padding if requested
    if let Some(pad_px) = args.pad {
        debug!("Adding {} px padding", pad_px);
        let pad = make_padding(print_width, pad_px);
        result = Some(append_bitmap(result, pad));
    }

    result.ok_or_else(|| {
        Box::new(PtouchError::SendFailed("No content to render".to_string()))
            as Box<dyn std::error::Error>
    })
}

/// Append a new bitmap to an existing one, or return the new bitmap if there
/// is no existing bitmap yet.
fn append_bitmap(existing: Option<LabelBitmap>, new: LabelBitmap) -> LabelBitmap {
    match existing {
        Some(prev) => prev.append(&new),
        None => new,
    }
}

/// Create a cut mark bitmap: a dashed vertical line.
///
/// The mark is 1 pixel wide with alternating black/white dots across the
/// tape height.
fn make_cutmark(print_width: u32) -> LabelBitmap {
    let mut bmp = LabelBitmap::new(1, print_width);
    for y in 0..print_width {
        // Alternating 2-pixel dashes
        if (y / 2) % 2 == 0 {
            bmp.set_pixel(0, y, true);
        }
    }
    bmp
}

/// Create a blank padding bitmap of the given width (in the print direction).
fn make_padding(print_width: u32, pad_px: u32) -> LabelBitmap {
    LabelBitmap::new(pad_px, print_width)
}

/// Send the label bitmap to the printer.
fn print_to_device(
    dev: &mut PtouchDevice,
    bitmap: &LabelBitmap,
    max_px: u16,
    args: &PrintArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let raster_lines = raster::bitmap_to_raster_lines(bitmap, max_px);

    let total_copies = args.copies.max(1);
    for copy_idx in 0..total_copies {
        let is_last = copy_idx == total_copies - 1;
        let chain_print = args.chain && is_last;

        debug!(
            "Printing copy {}/{} ({} raster lines, chain={})",
            copy_idx + 1,
            total_copies,
            raster_lines.len(),
            chain_print
        );

        dev.print_raster(&raster_lines, chain_print)?;
    }

    let tape_mm = bitmap.width() as f64 / 180.0 * 25.4;
    println!(
        "Printed {} cop{} ({:.1} mm of tape each)",
        total_copies,
        if total_copies == 1 { "y" } else { "ies" },
        tape_mm
    );

    Ok(())
}
