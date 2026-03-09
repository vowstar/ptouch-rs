// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Label rendering engine for Brother P-Touch printers.
//!
//! This crate provides bitmap rendering, text layout, image loading,
//! and raster conversion for label printing.

pub mod bitmap;
pub mod compose;
pub mod font;
pub mod image_loader;
pub mod raster;
pub mod text;

/// Errors produced by the render crate.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// An I/O error occurred.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// An image processing error occurred.
    #[error("Image error: {0}")]
    Image(#[from] image::ImageError),

    /// A text rendering error occurred.
    #[error("Text rendering error: {0}")]
    Text(String),

    /// Dimension mismatch during bitmap operations.
    #[error("Dimension mismatch: {0}")]
    DimensionMismatch(String),
}

/// Result type alias for render operations.
pub type Result<T> = std::result::Result<T, RenderError>;
