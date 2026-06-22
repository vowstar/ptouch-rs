// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Label rendering engine for Brother P-Touch printers.
//!
//! This crate provides bitmap rendering, text layout, image loading,
//! and raster conversion for label printing.

pub mod base64_bytes;
pub mod bitmap;
pub mod compose;
pub mod document;
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

    /// A layout document (de)serialization error occurred.
    #[error("Layout error: {0}")]
    Layout(String),
}

impl From<toml::ser::Error> for RenderError {
    fn from(e: toml::ser::Error) -> Self {
        RenderError::Layout(e.to_string())
    }
}

impl From<toml::de::Error> for RenderError {
    fn from(e: toml::de::Error) -> Self {
        RenderError::Layout(e.to_string())
    }
}

/// Result type alias for render operations.
pub type Result<T> = std::result::Result<T, RenderError>;
