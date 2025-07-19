//! Bot for posting movie frames to Bluesky at regular intervals.
//!
//! This bot reads JXL frames from a directory, converts them to JPEG with
//! automatic quality adjustment, and posts them to Bluesky on a schedule.
//! Frame progress is tracked to avoid duplicate posts.

mod bluesky;
mod config;
mod error;
mod frame_info;
mod frame_processing;

use anyhow::bail;
use log::*;
use tokio_schedule::{
    every,
    Job,
};

use crate::{
    bluesky::post_frame_task,
    config::{
        Config,
        POST_INTERVAL_SECONDS,
    },
};

/// Entry point - starts the frame posting bot.
///
/// Loads configuration from environment variables, authenticates with Bluesky,
/// and starts the posting loop. Runs indefinitely until interrupted.
#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    init_logging();
    dotenvy::dotenv().ok();

    // Check that the frames directory exists and has at least one frame.
    let frames_dir = config::FRAMES_DIR;
    if !std::path::Path::new(frames_dir).exists() {
        bail!("Frames directory '{}' does not exist", frames_dir);
    }

    let frame_count = frame_processing::get_total_frame_count().await?;
    if frame_count == 0 {
        bail!("No frames found in directory '{}'", frames_dir);
    }

    let config = Config::from_env()?;
    bluesky::initialize_agent(&config).await?;

    info!(
        "Starting frame posting bot for movie: {}",
        config.movie_name
    );

    let movie_name = config.movie_name.clone();

    if config.post_immediately {
        info!("Posting frames immediately on startup");
        post_frame_task(&movie_name).await;
    } else {
        info!("Will post frames every {} seconds", POST_INTERVAL_SECONDS);
    }

    every(POST_INTERVAL_SECONDS)
        .seconds()
        .perform(move || {
            let movie_name = movie_name.clone();
            async move {
                post_frame_task(&movie_name).await;
            }
        })
        .await;

    Ok(())
}

/// Set up logging with appropriate levels.
fn init_logging() {
    use env_logger::{
        Builder,
        Target,
    };
    use log::LevelFilter;

    Builder::new()
        .target(Target::Stdout)
        .filter_level(LevelFilter::Info)
        .filter_module("bsky_sdk", LevelFilter::Warn)
        .init();
}
