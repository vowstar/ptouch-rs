// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Command-line tool for Brother P-Touch label printers.
//!
//! Supports printing text labels, images, or combinations of both.
//! Can also export labels to image files (PNG, JPEG, BMP, etc.) for preview.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::process;

use clap::parser::ValueSource;
use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use log::debug;

use ptouch_core::device::{self, DeviceFlags, DeviceInfo};
use ptouch_core::error::PtouchError;
use ptouch_core::protocol::PrintQuality;
use ptouch_core::tape;
use ptouch_core::transport::PtouchDevice;

use ptouch_render::bitmap::LabelBitmap;
use ptouch_render::document::{self, LabelDocument};
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
// Print carries many options; it is constructed once at startup, so the size
// difference between subcommands does not matter here.
#[allow(clippy::large_enum_variant)]
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

    /// Print a saved layout file (.ptl). The layout is authoritative; ad-hoc
    /// content flags (text, --image, --font, --size, --align, --margin, --cut,
    /// --pad) are ignored with a warning.
    #[arg(short = 'l', long, value_name = "FILE")]
    layout: Option<String>,

    /// Set a layout placeholder value (repeatable): --set name=Alice
    #[arg(long = "set", value_name = "KEY=VALUE")]
    set: Vec<String>,

    /// Print one label per row of a CSV file ('-' for stdin); the header row
    /// names the placeholders. With --output, include '{n}' for the row number.
    #[arg(long, value_name = "FILE")]
    csv: Option<String>,

    /// List the placeholders a layout declares, then exit
    #[arg(long)]
    list_vars: bool,

    /// Render placeholders with no value as blank instead of erroring
    #[arg(long)]
    allow_missing: bool,

    /// Print an image file
    #[arg(short = 'i', long)]
    image: Option<String>,

    /// Binarization mode for images
    #[arg(long, value_enum, default_value = "auto")]
    binarize: BinarizeArg,

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

    /// Force tape width in pixels (use with -o for image export without printer)
    #[arg(short = 'w', long)]
    tape_width: Option<u32>,

    /// Add a cut mark
    #[arg(short = 'c', long)]
    cut: bool,

    /// Add padding in pixels
    #[arg(short = 'p', long)]
    pad: Option<u32>,

    /// Mirror the whole label left-right (horizontal). With --layout, the
    /// layout's saved flip wins and this is ignored with a warning.
    #[arg(long)]
    flip_h: bool,

    /// Mirror the whole label top-bottom (vertical). With --layout, the
    /// layout's saved flip wins and this is ignored with a warning.
    #[arg(long)]
    flip_v: bool,

    /// Skip final feed and cut (for chained labels)
    #[arg(long)]
    chain: bool,

    /// Cut before label
    #[arg(long)]
    precut: bool,

    /// Print quality (high and draft need a printer with quality modes)
    #[arg(long, value_enum, default_value = "standard")]
    quality: QualityArg,

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

#[derive(ValueEnum, Clone, Copy, Debug)]
enum QualityArg {
    Standard,
    High,
    Draft,
}

impl QualityArg {
    fn to_print_quality(self) -> PrintQuality {
        match self {
            QualityArg::Standard => PrintQuality::Standard,
            QualityArg::High => PrintQuality::HighRes,
            QualityArg::Draft => PrintQuality::Draft,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum BinarizeArg {
    Auto,
    Threshold,
    Dither,
}

impl BinarizeArg {
    fn to_binarize_mode(self) -> image_loader::BinarizeMode {
        match self {
            BinarizeArg::Auto => image_loader::BinarizeMode::Auto,
            BinarizeArg::Threshold => image_loader::BinarizeMode::Threshold,
            BinarizeArg::Dither => image_loader::BinarizeMode::Dither,
        }
    }
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

fn main() {
    let matches = Cli::command().get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(e) => e.exit(),
    };

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
            // When a layout drives the label, ad-hoc content flags do not
            // apply; collect the ones the user typed so we can warn.
            let ignored = if args.layout.is_some() {
                ignored_content_flags(&matches)
            } else {
                Vec::new()
            };
            if let Err(e) = execute_print(&args, &ignored) {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        }
    }
}

/// Ad-hoc content flags overridden when `--layout` is set. Keep in sync with
/// `PrintArgs`; the `content_flag_ids_resolve` test guards against renames.
const CONTENT_FLAG_IDS: &[&str] = &[
    "text", "image", "font", "size", "align", "margin", "cut", "pad", "flip_h", "flip_v",
];

/// Return the display names of content flags the user explicitly passed on the
/// command line (defaults do not count), for the warn-and-ignore message.
fn ignored_content_flags(matches: &ArgMatches) -> Vec<String> {
    let Some(sub) = matches.subcommand_matches("print") else {
        return Vec::new();
    };
    CONTENT_FLAG_IDS
        .iter()
        .filter(|id| sub.value_source(id) == Some(ValueSource::CommandLine))
        .map(|id| {
            if *id == "text" {
                "TEXT".to_string()
            } else {
                // clap derives long flags with hyphens (flip_h -> --flip-h).
                format!("--{}", id.replace('_', "-"))
            }
        })
        .collect()
}

/// Parse `--set KEY=VALUE` arguments into a map. Errors on an entry with no `=`.
fn parse_set_args(set: &[String]) -> Result<BTreeMap<String, String>, Box<dyn std::error::Error>> {
    let mut values = BTreeMap::new();
    for entry in set {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| format!("invalid --set '{}' (expected KEY=VALUE)", entry))?;
        values.insert(key.to_string(), value.to_string());
    }
    Ok(values)
}

/// Check provided values against the placeholders a layout declares.
///
/// Warns once about values that the layout does not use, and (unless
/// `allow_missing`) errors when a declared placeholder has no value.
fn validate_vars(
    declared: &[String],
    provided: &BTreeSet<String>,
    allow_missing: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let declared_set: BTreeSet<&String> = declared.iter().collect();

    let unused: Vec<&String> = provided
        .iter()
        .filter(|name| !declared_set.contains(name))
        .collect();
    if !unused.is_empty() {
        let names: Vec<&str> = unused.iter().map(|s| s.as_str()).collect();
        eprintln!(
            "WARN: value(s) not used by the layout: {}",
            names.join(", ")
        );
    }

    if !allow_missing {
        let missing: Vec<&str> = declared
            .iter()
            .filter(|name| !provided.contains(*name))
            .map(|s| s.as_str())
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "missing value(s) for placeholder(s): {} (pass --set or --allow-missing)",
                missing.join(", ")
            )
            .into());
        }
    }
    Ok(())
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

    // init() already called get_status() internally; use that result.
    let status = dev
        .status()
        .ok_or_else(|| PtouchError::StatusError("No status available after init".to_string()))?
        .clone();

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
    let tapes = tape::supported_tapes(dpi);
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

/// Build a label from a layout file or ad-hoc text/images, then print or save.
fn execute_print(args: &PrintArgs, ignored: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.tape_width == Some(0) {
        eprintln!("Error: --tape-width must be greater than 0");
        process::exit(1);
    }

    // Layout-only modifiers make no sense without a layout.
    if args.layout.is_none()
        && (!args.set.is_empty() || args.csv.is_some() || args.list_vars || args.allow_missing)
    {
        eprintln!("Error: --set, --csv, --list-vars, and --allow-missing require --layout");
        process::exit(1);
    }

    if let Some(layout_path) = args.layout.as_deref() {
        if !ignored.is_empty() {
            eprintln!("WARN: --layout is set; ignoring: {}", ignored.join(", "));
        }
        return print_layout(args, layout_path);
    }

    // Validate arguments
    if args.text.is_empty() && args.image.is_none() {
        eprintln!("Error: nothing to print (provide text, --image, or --layout)");
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
            // init() already called get_status() internally
            let width = dev.tape_width_px().ok_or_else(|| {
                PtouchError::StatusError("Could not determine tape width".to_string())
            })?;
            let max = dev.max_px();
            debug!("Printer tape width: {} px, max: {} px", width, max);
            (u32::from(width), max, Some(dev))
        };

    // Whole-label mirroring applies once, after the label is composed.
    let bitmap = build_label(args, print_width)?.mirrored(args.flip_h, args.flip_v);
    emit_label(&bitmap, args, max_px, device.as_mut())?;

    if let Some(dev) = device {
        dev.close()?;
    }

    Ok(())
}

/// Load a `.ptl` layout, render it, and either print it or save as an image.
fn print_layout(args: &PrintArgs, layout_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(layout_path)?;
    let mut doc = LabelDocument::from_toml_str(&text)?;

    if args.list_vars {
        for name in doc.placeholders() {
            println!("{}", name);
        }
        return Ok(());
    }

    if let Some(csv_path) = args.csv.as_deref() {
        return print_layout_batch(args, doc, csv_path);
    }

    // Fill placeholders from --set values, rejecting missing ones by default.
    let values = parse_set_args(&args.set)?;
    let provided: BTreeSet<String> = values.keys().cloned().collect();
    validate_vars(&doc.placeholders(), &provided, args.allow_missing)?;
    doc.apply_values(&values);

    let (print_width, max_px, mut device) = resolve_layout_target(args, &doc)?;
    let bitmap = render_layout(&doc, print_width)?;
    emit_label(&bitmap, args, max_px, device.as_mut())?;

    if let Some(dev) = device {
        dev.close()?;
    }

    Ok(())
}

/// Print one label per CSV row, substituting the header columns (plus any
/// `--set` constants) into the layout placeholders.
fn print_layout_batch(
    args: &PrintArgs,
    doc: LabelDocument,
    csv_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(output) = &args.output
        && !output.contains("{n}")
    {
        eprintln!("Error: with --csv, --output must contain '{{n}}' (e.g. label-{{n}}.png)");
        process::exit(1);
    }

    let base = parse_set_args(&args.set)?;
    let reader: Box<dyn Read> = if csv_path == "-" {
        Box::new(io::stdin())
    } else {
        Box::new(File::open(csv_path)?)
    };
    let mut rdr = csv::Reader::from_reader(reader);
    let headers: Vec<String> = rdr.headers()?.iter().map(|s| s.to_string()).collect();

    // Validate the placeholders against the CSV columns plus any --set keys.
    let mut provided: BTreeSet<String> = headers.iter().cloned().collect();
    provided.extend(base.keys().cloned());
    validate_vars(&doc.placeholders(), &provided, args.allow_missing)?;

    let (print_width, max_px, mut device) = resolve_layout_target(args, &doc)?;

    let mut count = 0usize;
    for record in rdr.records() {
        let record = record?;
        let row: Vec<String> = record.iter().map(|s| s.to_string()).collect();
        let values = build_row_values(&base, &headers, &row);

        let mut row_doc = doc.clone();
        row_doc.apply_values(&values);
        let bitmap = render_layout(&row_doc, print_width)?;

        count += 1;
        if let Some(output) = &args.output {
            let path = output.replace("{n}", &count.to_string());
            bitmap.save(Path::new(&path))?;
            println!("Saved row {} to '{}'", count, path);
        } else if let Some(dev) = device.as_mut() {
            print_to_device(dev, &bitmap, max_px, args)?;
        } else {
            eprintln!("Error: no output destination (use --output or connect a printer)");
            process::exit(1);
        }
    }

    if let Some(dev) = device {
        dev.close()?;
    }

    if count == 0 {
        eprintln!("WARN: CSV had no data rows; nothing printed");
    } else {
        println!("Processed {} row(s)", count);
    }
    Ok(())
}

/// Merge `--set` constants with one CSV row's columns (row values win).
fn build_row_values(
    base: &BTreeMap<String, String>,
    headers: &[String],
    record: &[String],
) -> BTreeMap<String, String> {
    let mut values = base.clone();
    for (header, value) in headers.iter().zip(record.iter()) {
        values.insert(header.clone(), value.clone());
    }
    values
}

/// Render a (placeholder-resolved) layout document to a single bitmap.
fn render_layout(
    doc: &LabelDocument,
    print_width: u32,
) -> Result<LabelBitmap, Box<dyn std::error::Error>> {
    let mut renderer = TextRenderer::new();
    let bitmap = document::render_elements(
        &doc.elements,
        print_width,
        &doc.font_name,
        doc.font_margin,
        &mut renderer,
    )?
    .ok_or_else(|| PtouchError::SendFailed("layout produced no output".to_string()))?;
    // The layout's saved whole-label flip is applied after composition.
    Ok(bitmap.mirrored(doc.flip_h, doc.flip_v))
}

/// Resolve the print width, max pixels, and optional device for a layout.
///
///   - `--tape-width`: forced PNG width (requires `--output`)
///   - `--output` only: PNG export at the saved width, no printer needed
///   - otherwise: print at the printer's actual width, warn on mismatch
fn resolve_layout_target(
    args: &PrintArgs,
    doc: &LabelDocument,
) -> Result<(u32, u16, Option<PtouchDevice>), Box<dyn std::error::Error>> {
    // Offline export renders at 180 dpi; printing uses the printer's own
    // status-derived width, so a 360 dpi printer just triggers the refit path.
    let saved_px = tape::find_tape(doc.tape_width_mm, 180).map(|t| u32::from(t.pixels));

    if let Some(w) = args.tape_width {
        if args.output.is_none() {
            eprintln!("Error: --tape-width requires --output");
            process::exit(1);
        }
        Ok((w, w as u16, None))
    } else if args.output.is_some() {
        let w = saved_px.ok_or_else(|| {
            PtouchError::StatusError("unknown saved tape width; pass --tape-width".to_string())
        })?;
        Ok((w, w as u16, None))
    } else {
        let mut dev = PtouchDevice::open_first()?;
        dev.init()?;
        let printer_px = u32::from(dev.tape_width_px().ok_or_else(|| {
            PtouchError::StatusError("Could not determine tape width".to_string())
        })?);
        if saved_px.is_some_and(|s| s != printer_px) {
            let printer_mm = dev.status().map(|s| s.media_width).unwrap_or(0);
            if printer_mm > 0 {
                eprintln!(
                    "WARN: layout saved for {}mm, printer has {}mm; refitting to printer tape",
                    doc.tape_width_mm, printer_mm
                );
            } else {
                eprintln!(
                    "WARN: layout saved for {}mm tape; refitting to the printer tape",
                    doc.tape_width_mm
                );
            }
        }
        let max = dev.max_px();
        Ok((printer_px, max, Some(dev)))
    }
}

/// Save a rendered label to an image file or print it to the device.
fn emit_label(
    bitmap: &LabelBitmap,
    args: &PrintArgs,
    max_px: u16,
    device: Option<&mut PtouchDevice>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(ref output_path) = args.output {
        bitmap.save(Path::new(output_path))?;
        let dpi = device.as_ref().map_or(180, |d| d.device_info().dpi);
        let tape_mm = bitmap.width() as f64 / f64::from(dpi) * 25.4;
        println!(
            "Saved to '{}' ({}x{} px, {:.1} mm of tape)",
            output_path,
            bitmap.width(),
            bitmap.height(),
            tape_mm
        );
    } else if let Some(dev) = device {
        print_to_device(dev, bitmap, max_px, args)?;
    } else {
        eprintln!("Error: no output destination (use --output or connect a printer)");
        process::exit(1);
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
        let options = image_loader::ImageLoadOptions {
            binarize: args.binarize.to_binarize_mode(),
            target_height: Some(print_width),
            ..image_loader::ImageLoadOptions::default()
        };
        let img_bitmap = image_loader::load_image(Path::new(img_path), &options)?;
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
        // Chain intermediate copies (no cut between copies).
        // Last copy: chain only if user requested --chain.
        // Chain intermediate copies; last copy follows user's --chain flag
        let chain_print = args.chain || !is_last;

        debug!(
            "Printing copy {}/{} ({} raster lines, chain={})",
            copy_idx + 1,
            total_copies,
            raster_lines.len(),
            chain_print
        );

        dev.print_raster(
            &raster_lines,
            chain_print,
            args.precut,
            args.quality.to_print_quality(),
        )?;
    }

    let tape_mm = bitmap.width() as f64 / f64::from(dev.device_info().dpi) * 25.4;
    println!(
        "Printed {} cop{} ({:.1} mm of tape each)",
        total_copies,
        if total_copies == 1 { "y" } else { "ies" },
        tape_mm
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn print_matches(argv: &[&str]) -> ArgMatches {
        Cli::command().get_matches_from(argv)
    }

    #[test]
    fn test_content_flag_ids_resolve() {
        let cmd = Cli::command();
        let print = cmd.find_subcommand("print").expect("print subcommand");
        for id in CONTENT_FLAG_IDS {
            assert!(
                print.get_arguments().any(|a| a.get_id() == *id),
                "content flag id '{}' not found in print args",
                id
            );
        }
    }

    #[test]
    fn test_typed_content_flags_are_reported() {
        let matches = print_matches(&[
            "ptouch", "print", "Hello", "-s", "24", "--flip-h", "-l", "x.ptl",
        ]);
        let ignored = ignored_content_flags(&matches);
        assert!(ignored.contains(&"TEXT".to_string()));
        assert!(ignored.contains(&"--size".to_string()));
        // Underscore ids are shown with hyphens, matching the real flag name.
        assert!(ignored.contains(&"--flip-h".to_string()));
        assert!(!ignored.contains(&"--font".to_string()));
        assert!(!ignored.contains(&"--flip-v".to_string()));
    }

    #[test]
    fn test_defaulted_flags_are_not_reported() {
        // Only --layout is given; defaults (font, align, ...) must not warn.
        let matches = print_matches(&["ptouch", "print", "-l", "x.ptl"]);
        let ignored = ignored_content_flags(&matches);
        assert!(
            ignored.is_empty(),
            "unexpected ignored flags: {:?}",
            ignored
        );
    }

    #[test]
    fn test_parse_set_args_ok() {
        let set = vec!["name=Alice".to_string(), "id=A=1".to_string()];
        let values = parse_set_args(&set).unwrap();
        assert_eq!(values.get("name").map(String::as_str), Some("Alice"));
        // Only the first '=' splits, so values may contain '='.
        assert_eq!(values.get("id").map(String::as_str), Some("A=1"));
    }

    #[test]
    fn test_parse_set_args_rejects_missing_eq() {
        assert!(parse_set_args(&["bogus".to_string()]).is_err());
    }

    #[test]
    fn test_validate_vars_missing_errors() {
        let declared = vec!["name".to_string(), "id".to_string()];
        let provided: BTreeSet<String> = ["name".to_string()].into_iter().collect();
        assert!(validate_vars(&declared, &provided, false).is_err());
        // allow_missing turns the error off.
        assert!(validate_vars(&declared, &provided, true).is_ok());
    }

    #[test]
    fn test_validate_vars_unused_is_ok() {
        let declared = vec!["name".to_string()];
        let provided: BTreeSet<String> = ["name".to_string(), "extra".to_string()]
            .into_iter()
            .collect();
        // "extra" is unused (warns) but not an error.
        assert!(validate_vars(&declared, &provided, false).is_ok());
    }

    #[test]
    fn test_build_row_values_row_overrides_base() {
        let mut base = BTreeMap::new();
        base.insert("name".to_string(), "Default".to_string());
        base.insert("dept".to_string(), "Eng".to_string());
        let headers = vec!["name".to_string(), "id".to_string()];
        let record = vec!["Alice".to_string(), "A001".to_string()];
        let values = build_row_values(&base, &headers, &record);
        assert_eq!(values.get("name").map(String::as_str), Some("Alice")); // row wins
        assert_eq!(values.get("id").map(String::as_str), Some("A001")); // from row
        assert_eq!(values.get("dept").map(String::as_str), Some("Eng")); // base kept
    }

    #[test]
    fn test_output_n_token_replacement() {
        assert_eq!("label-{n}.png".replace("{n}", "3"), "label-3.png");
    }
}
