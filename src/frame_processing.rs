//! JPEG loading and recompression with automatic quality optimization.

use std::{
    io::Cursor,
    sync::OnceLock,
};

use anyhow::{
    bail,
    Context,
};
use image::{
    DynamicImage,
    GenericImageView,
};
use log::*;

use crate::{
    config::{
        FRAMES_DIR,
        JPEG_QUALITY_STEP,
        MAX_JPEG_SIZE,
        MIN_JPEG_QUALITY,
    },
    error::FrameError,
};

/// Cached total frame count to avoid repeated directory scans.
static FRAME_COUNT: OnceLock<u32> = OnceLock::new();

/// Image dimensions in pixels.
#[derive(Debug)]
pub struct FrameDimensions {
    pub width: u32,
    pub height: u32,
}

/// A processed frame ready for upload.
#[derive(Debug)]
pub struct ProcessedFrame {
    pub jpeg_data: Vec<u8>,
    pub dimensions: FrameDimensions,
    pub quality_used: Option<u8>, // None if original was used
}

/// Get total frame count, using cached value if available.
pub async fn get_total_frame_count() -> anyhow::Result<u32> {
    if let Some(&count) = FRAME_COUNT.get() {
        return Ok(count);
    }

    let count = count_frame_files().await?;
    FRAME_COUNT
        .set(count)
        .map_err(|_| anyhow::anyhow!("Failed to cache frame count"))?;

    debug!("Total frames detected: {}", count);
    Ok(count)
}

/// Count JPEG files in the frames directory.
async fn count_frame_files() -> anyhow::Result<u32> {
    let mut entries = tokio::fs::read_dir(FRAMES_DIR)
        .await
        .with_context(|| format!("Failed to read frames directory: {}", FRAMES_DIR))?;
    let mut count = 0;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("jpg") {
            count += 1;
        }
    }

    Ok(count)
}

/// Load JPEG frame and recompress only if needed for size optimization.
///
/// Takes a frame number, loads the corresponding JPEG file. If the file is already
/// within the size limit, returns it directly. Otherwise, recompresses with quality
/// optimization to meet the size requirements.
pub async fn get_frame_as_jpeg(current_frame: u32) -> anyhow::Result<ProcessedFrame> {
    validate_frame_number(current_frame)?;

    let frame_path = format!("{}/{}.jpg", FRAMES_DIR, current_frame);
    ensure_frame_exists(&frame_path).await?;

    let jpeg_data = tokio::fs::read(&frame_path)
        .await
        .with_context(|| format!("Failed to read frame file: {}", frame_path))?;

    let original_size = jpeg_data.len();
    debug!(
        "Frame {} original size: {} bytes",
        current_frame, original_size
    );

    // If already within size limit, return original data directly
    if original_size <= MAX_JPEG_SIZE {
        debug!(
            "Frame {} already within size limit, using original",
            current_frame
        );

        let result =
            tokio::task::spawn_blocking(move || get_image_dimensions(jpeg_data, current_frame))
                .await
                .with_context(|| {
                    format!(
                        "Task panicked while getting dimensions for frame {}",
                        current_frame
                    )
                })??;

        return Ok(result);
    }

    // File is too large, needs recompression
    debug!(
        "Frame {} too large ({}), recompressing",
        current_frame, original_size
    );

    let result =
        tokio::task::spawn_blocking(move || process_jpeg_recompression(jpeg_data, current_frame))
            .await
            .with_context(|| {
                format!("Task panicked while recompressing frame {}", current_frame)
            })??;

    debug!(
        "Frame {} recompressed successfully (quality: {:?})",
        current_frame, result.quality_used
    );
    Ok(result)
}

/// Warn if frame number seems unusual.
fn validate_frame_number(frame: u32) -> anyhow::Result<()> {
    if frame == 0 {
        warn!("Frame number 0 provided - this might be unexpected");
    }
    Ok(())
}

/// Check if frame file exists before processing.
async fn ensure_frame_exists(path: &str) -> anyhow::Result<()> {
    if !tokio::fs::try_exists(path).await? {
        bail!("Frame file does not exist: {}", path);
    }
    Ok(())
}

/// Get image dimensions from JPEG data without recompression.
fn get_image_dimensions(jpeg_data: Vec<u8>, frame_num: u32) -> anyhow::Result<ProcessedFrame> {
    trace!(
        "Getting dimensions for frame {} without recompression",
        frame_num
    );

    let image = image::load_from_memory(&jpeg_data)
        .with_context(|| format!("Failed to decode JPEG for frame {}", frame_num))?;

    let (width, height) = image.dimensions();
    debug!("Frame {} dimensions: {}x{}", frame_num, width, height);

    Ok(ProcessedFrame {
        jpeg_data,
        dimensions: FrameDimensions { width, height },
        quality_used: None, // Original image used as-is
    })
}

/// Recompress JPEG with quality optimization to meet size requirements.
fn process_jpeg_recompression(
    jpeg_data: Vec<u8>,
    frame_num: u32,
) -> anyhow::Result<ProcessedFrame> {
    trace!("Decoding JPEG for recompression, frame {}", frame_num);
    let image = image::load_from_memory(&jpeg_data)
        .with_context(|| format!("Failed to decode JPEG for frame {}", frame_num))?;

    let (width, height) = image.dimensions();
    debug!("Frame {} dimensions: {}x{}", frame_num, width, height);

    // Convert to RGB8 to ensure consistent format for recompression
    trace!("Converting image to RGB8 format");
    let rgb_image = DynamicImage::ImageRgb8(image.to_rgb8());

    let (optimized_data, quality_used) = compress_to_jpeg(&rgb_image, frame_num)?;

    Ok(ProcessedFrame {
        jpeg_data: optimized_data,
        dimensions: FrameDimensions { width, height },
        quality_used: Some(quality_used),
    })
}

/// Compress image to JPEG under the size limit.
///
/// Iteratively reduces JPEG quality until the file size is under the limit.
/// Starts at maximum quality and works down in steps. Fails if even minimum
/// quality produces a file that's too large.
fn compress_to_jpeg(image: &DynamicImage, frame_num: u32) -> anyhow::Result<(Vec<u8>, u8)> {
    let mut quality = 100u8;
    let mut buffer = Vec::with_capacity(MAX_JPEG_SIZE);
    let mut attempts = 0;

    debug!(
        "Starting JPEG encoding with quality optimization for frame {}",
        frame_num
    );

    loop {
        attempts += 1;
        buffer.clear();

        trace!("Attempt {}: Encoding with quality {}", attempts, quality);

        let mut cursor = Cursor::new(&mut buffer);
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, quality);

        image.write_with_encoder(encoder).with_context(|| {
            format!(
                "Failed to encode frame {} to JPEG at quality {}",
                frame_num, quality
            )
        })?;

        let buffer_size = buffer.len();
        debug!("JPEG encoded at quality {}: {} bytes", quality, buffer_size);

        if buffer_size <= MAX_JPEG_SIZE {
            debug!(
                "Successfully recompressed frame {} to JPEG: {} bytes at quality {}",
                frame_num, buffer_size, quality
            );
            return Ok((buffer, quality));
        }

        if quality <= MIN_JPEG_QUALITY {
            return Err(FrameError::CompressionFailed {
                frame: frame_num,
                max_size: MAX_JPEG_SIZE as f64 / 1_000_000.0,
            }
            .into());
        }

        let old_quality = quality;
        quality = quality.saturating_sub(JPEG_QUALITY_STEP);
        debug!(
            "Buffer too large ({} bytes), reducing quality from {} to {}",
            buffer_size, old_quality, quality
        );
    }
}
