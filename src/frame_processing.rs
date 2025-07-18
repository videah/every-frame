//! JXL to JPEG conversion with automatic quality optimization.

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
    ImageDecoder,
    ImageEncoder,
};
use jxl_oxide::integration::JxlDecoder;
use log::*;
use tokio::fs::File;

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
    pub quality_used: u8,
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

/// Count JXL files in the frames directory.
async fn count_frame_files() -> anyhow::Result<u32> {
    let mut entries = tokio::fs::read_dir(FRAMES_DIR)
        .await
        .with_context(|| format!("Failed to read frames directory: {}", FRAMES_DIR))?;
    let mut count = 0;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("jxl") {
            count += 1;
        }
    }

    Ok(count)
}

/// Convert JXL frame to JPEG with size optimization.
///
/// Takes a frame number, loads the corresponding JXL file, and converts
/// it to JPEG format. Automatically adjusts quality to keep the file
/// under the size limit while maintaining the best possible image quality.
/// The conversion happens in a blocking task to avoid blocking the async runtime.
pub async fn get_frame_as_jpeg(current_frame: u32) -> anyhow::Result<ProcessedFrame> {
    validate_frame_number(current_frame)?;

    let frame_path = format!("{}/{}.jxl", FRAMES_DIR, current_frame);
    ensure_frame_exists(&frame_path).await?;

    debug!("Converting frame {} to JPEG", current_frame);

    let file = File::open(&frame_path)
        .await
        .with_context(|| format!("Failed to open frame file: {}", frame_path))?;
    let std_file = file.into_std().await;

    let result = tokio::task::spawn_blocking(move || process_jxl_to_jpeg(std_file, current_frame))
        .await
        .with_context(|| format!("Task panicked while processing frame {}", current_frame))??;

    debug!(
        "Frame {} converted successfully (quality: {})",
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

/// Decode JXL and convert to JPEG with quality optimization.
///
/// Handles the actual image processing work: decoding the JXL file,
/// extracting any ICC color profile, converting to RGB, and then
/// compressing to JPEG. Preserves color information when possible.
fn process_jxl_to_jpeg(file: std::fs::File, frame_num: u32) -> anyhow::Result<ProcessedFrame> {
    trace!("Initializing JXL decoder for frame {}", frame_num);
    let mut decoder = JxlDecoder::new(file)
        .with_context(|| format!("Failed to create JXL decoder for frame {}", frame_num))?;

    trace!("Extracting ICC profile for frame {}", frame_num);
    let icc_profile = decoder
        .icc_profile()
        .with_context(|| format!("Failed to get ICC profile for frame {}", frame_num))?;

    if icc_profile.is_some() {
        trace!("ICC profile found and will be preserved");
    } else {
        trace!("No ICC profile found");
    }

    debug!("Decoding JXL image for frame {}", frame_num);
    let image = DynamicImage::from_decoder(decoder)
        .with_context(|| format!("Failed to decode JXL for frame {}", frame_num))?;

    trace!("Converting image to RGB8 format");
    let rgb_image = DynamicImage::ImageRgb8(image.to_rgb8());
    let (width, height) = rgb_image.dimensions();

    debug!(
        "Image dimensions: {}x{} for frame {}",
        width, height, frame_num
    );

    let (jpeg_data, quality_used) = compress_to_jpeg(&rgb_image, icc_profile, frame_num)?;

    Ok(ProcessedFrame {
        jpeg_data,
        dimensions: FrameDimensions { width, height },
        quality_used,
    })
}

/// Compress image to JPEG under the size limit.
///
/// Iteratively reduces JPEG quality until the file size is under the limit.
/// Starts at maximum quality and works down in steps. Preserves ICC color
/// profiles when available. Fails if even minimum quality produces a file
/// that's too large.
fn compress_to_jpeg(
    image: &DynamicImage,
    icc_profile: Option<Vec<u8>>,
    frame_num: u32,
) -> anyhow::Result<(Vec<u8>, u8)> {
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
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, quality);

        if let Some(ref icc) = icc_profile {
            trace!("Setting ICC profile for JPEG encoder");
            encoder
                .set_icc_profile(icc.clone())
                .context("Failed to set ICC profile when encoding JXL to JPEG")?;
        }

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
                "Successfully encoded frame {} to JPEG: {} bytes at quality {}",
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
