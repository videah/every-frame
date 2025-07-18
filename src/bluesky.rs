//! Bluesky authentication and posting operations.

use std::num::NonZeroU64;

use anyhow::Context;
use bsky_sdk::{
    agent::config::{
        Config as BskyConfig,
        FileStore,
    },
    api::{
        app::bsky::{
            embed::{
                defs::{
                    AspectRatio,
                    AspectRatioData,
                },
                images::{
                    self,
                    ImageData,
                },
            },
            feed::post,
        },
        com::atproto::repo::upload_blob,
        types::{
            string::Datetime,
            Union,
        },
    },
    BskyAgent,
};
use ipld_core::ipld::Ipld;
use log::*;

use crate::{
    config::{
        Config,
        FRAME_DATA_FILE,
        MAX_RETRIES,
        RETRY_DELAY,
        SESSION_FILE,
    },
    frame_info::FrameInfo,
    frame_processing::{
        get_frame_as_jpeg,
        get_total_frame_count,
        FrameDimensions,
    },
};

/// Create and authenticate a Bluesky agent.
///
/// Sets up the agent with the provided credentials, performs initial
/// authentication, and saves the session for future use.
pub async fn initialize_agent(config: &Config) -> anyhow::Result<BskyAgent> {
    let agent = BskyAgent::builder().build().await?;
    agent
        .login(&config.identifier, &config.app_password)
        .await?;

    agent
        .to_config()
        .await
        .save(&FileStore::new(SESSION_FILE))
        .await?;

    info!("Successfully authenticated with Bluesky");
    Ok(agent)
}

/// Post a frame with retry logic.
///
/// Attempts to post a frame up to MAX_RETRIES times, with a delay
/// between attempts. This handles temporary network issues and
/// transient failures gracefully.
pub async fn post_frame_task(movie_name: &str) {
    for attempt in 1..=MAX_RETRIES {
        match post_frame(movie_name).await {
            Ok(_) => {
                info!("Frame posted successfully!");
                return;
            }
            Err(e) => {
                error!(
                    "Attempt {}/{} failed to post frame: {}",
                    attempt, MAX_RETRIES, e
                );
                if attempt < MAX_RETRIES {
                    warn!("Retrying in {} seconds...", RETRY_DELAY.as_secs());
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            }
        }
    }
    error!("Failed to post frame after {} attempts", MAX_RETRIES);
}

/// Post a single frame to Bluesky.
///
/// Orchestrates the entire posting process: loads the current frame info,
/// converts the frame to JPEG, uploads it to Bluesky, creates a post with
/// the image, and updates the frame counter for next time. Also saves the
/// session after successful posting to maintain authentication.
pub async fn post_frame(movie_name: &str) -> anyhow::Result<()> {
    info!("Preparing to post a frame...");

    let agent = load_agent().await?;
    let total_frames = get_total_frame_count().await?;
    let mut frame_info = FrameInfo::load_or_create(FRAME_DATA_FILE, total_frames, 1)?;

    let processed_frame = get_frame_as_jpeg(frame_info.current_frame).await?;
    let blob = upload_frame_blob(&agent, processed_frame.jpeg_data).await?;

    let post_data = create_post_data(
        movie_name,
        &frame_info,
        total_frames,
        blob,
        &processed_frame.dimensions,
    )?;

    agent
        .create_record(post_data)
        .await
        .context("Failed to create post record")?;

    // Save session after successful post
    agent
        .to_config()
        .await
        .save(&FileStore::new(SESSION_FILE))
        .await
        .context("Failed to save session after posting")?;

    frame_info.increment(FRAME_DATA_FILE)?;

    info!(
        "Successfully posted frame {}/{}",
        frame_info.current_frame, total_frames
    );
    Ok(())
}

/// Load authenticated agent from saved session.
async fn load_agent() -> anyhow::Result<BskyAgent> {
    BskyAgent::builder()
        .config(BskyConfig::load(&FileStore::new(SESSION_FILE)).await?)
        .build()
        .await
        .context("Failed to load agent from session")
}

/// Upload JPEG data to Bluesky.
async fn upload_frame_blob(
    agent: &BskyAgent,
    jpeg_data: Vec<u8>,
) -> anyhow::Result<upload_blob::OutputData> {
    agent
        .api
        .com
        .atproto
        .repo
        .upload_blob(jpeg_data)
        .await
        .map(|response| response.data)
        .context("Failed to upload frame blob")
}

/// Create post data with image and metadata.
///
/// Builds the complete post structure including the image embed,
/// alt text description, and aspect ratio information.
fn create_post_data(
    movie_name: &str,
    frame_info: &FrameInfo,
    total_frames: u32,
    blob: upload_blob::OutputData,
    dimensions: &FrameDimensions,
) -> anyhow::Result<post::RecordData> {
    let images =
        vec![ImageData {
        alt: format!(
            "A frame from the movie '{movie_name}', specifically frame {} of {total_frames}",
            frame_info.current_frame
        ),
        image: blob.blob,
        aspect_ratio: Some(AspectRatio {
            data: AspectRatioData {
                width: NonZeroU64::new(dimensions.width as u64)
                    .context("Invalid width dimension")?,
                height: NonZeroU64::new(dimensions.height as u64)
                    .context("Invalid height dimension")?,
            },
            extra_data: Ipld::Null,
        }),
    }.into()];

    let embed = Some(Union::Refs(post::RecordEmbedRefs::AppBskyEmbedImagesMain(
        Box::new(images::MainData { images }.into()),
    )));

    Ok(post::RecordData {
        created_at: Datetime::now(),
        embed,
        entities: None,
        facets: None,
        labels: None,
        langs: None,
        reply: None,
        tags: None,
        text: String::new(),
    })
}
