//! Configuration constants and environment variable handling.

use std::env;

use anyhow::Context;

/// Maximum JPEG file size in bytes before compression quality is reduced.
pub const MAX_JPEG_SIZE: usize = 1_000_000;

/// Minimum JPEG quality setting before giving up on compression.
pub const MIN_JPEG_QUALITY: u8 = 10;

/// Quality reduction step size when file is too large.
pub const JPEG_QUALITY_STEP: u8 = 5;

/// Directory containing JXL frame files.
pub const FRAMES_DIR: &str = "frames";

/// File storing the Bluesky session data.
pub const SESSION_FILE: &str = "config/session.toml";

/// File storing frame posting progress.
pub const FRAME_DATA_FILE: &str = "config/frame_data.toml";

/// Seconds between frame posts.
pub const POST_INTERVAL_SECONDS: u32 = 1800;

/// Maximum retry attempts for failed posts.
pub const MAX_RETRIES: u32 = 3;

/// Delay between retry attempts.
pub const RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(5);

/// Bot configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// Bluesky account identifier
    pub identifier: String,
    /// Bluesky app password
    pub app_password: String,
    /// Movie name for generating alt text
    pub movie_name: String,
}

impl Config {
    /// Load configuration from environment variables.
    ///
    /// Expects BLUESKY_IDENTIFIER, BLUESKY_APP_PASSWORD, and MOVIE_NAME
    /// to be set in the environment.
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            identifier: env::var("BLUESKY_IDENTIFIER")
                .context("Missing BLUESKY_IDENTIFIER environment variable")?,
            app_password: env::var("BLUESKY_APP_PASSWORD")
                .context("Missing BLUESKY_APP_PASSWORD environment variable")?,
            movie_name: env::var("MOVIE_NAME")
                .context("Missing MOVIE_NAME environment variable")?,
        })
    }
}
