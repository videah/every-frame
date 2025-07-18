//! Error types for frame processing operations.

use thiserror::Error;

/// Errors that can occur during frame processing.
#[derive(Error, Debug)]
pub enum FrameError {
    #[error("Failed to compress frame {frame} to under {max_size}MB at minimum quality")]
    CompressionFailed { frame: u32, max_size: f64 },
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image processing error: {0}")]
    Image(#[from] image::ImageError),
}
