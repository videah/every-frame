//! Frame tracking and persistence for movie posting progress.
//!
//! This module handles tracking which frame should be posted next and persisting
//! that information to disk so the bot can resume where it left off after restarts.
//! Frame numbering is 1-based to match typical movie frame conventions.

use std::{
    fs,
    io,
    path::Path,
};

use anyhow::{
    Context,
    Result,
};
use log::*;
use serde::{
    Deserialize,
    Serialize,
};

/// Tracks current posting progress through a movie's frames.
///
/// Maintains the total number of frames available and which frame
/// should be posted next. Persists this information to a TOML file
/// so posting can resume after restarts. Frame numbering is 1-based,
/// meaning the first frame is frame 1, not frame 0.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct FrameInfo {
    /// Total number of frames in the movie
    pub total_frames: u32,
    /// Next frame number to post (1-based indexing)
    pub current_frame: u32,
}

impl FrameInfo {
    /// Create a new FrameInfo with validation.
    ///
    /// Ensures that current_frame is within valid bounds (1 to total_frames).
    /// If total_frames is 0, current_frame is set to 0 as a special case.
    pub fn new(total_frames: u32, current_frame: u32) -> Result<Self> {
        if total_frames == 0 {
            warn!("Creating FrameInfo with zero total frames");
            return Ok(Self {
                total_frames: 0,
                current_frame: 0,
            });
        }

        if current_frame == 0 {
            return Err(anyhow::anyhow!(
                "Frame numbering is 1-based, current_frame cannot be 0 when total_frames > 0"
            ));
        }

        if current_frame > total_frames {
            return Err(anyhow::anyhow!(
                "Current frame {} exceeds total frames {}",
                current_frame,
                total_frames
            ));
        }

        Ok(Self {
            total_frames,
            current_frame,
        })
    }

    /// Advance to the next frame and save progress to disk.
    ///
    /// Increments current_frame by 1, wrapping back to 1 when reaching the end.
    /// This creates an infinite loop through all frames. Automatically saves
    /// the updated state to the specified file after incrementing.
    pub fn increment<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        if self.total_frames == 0 {
            warn!("Cannot increment frame when total_frames is 0");
            return Ok(());
        }

        let old_frame = self.current_frame;
        self.current_frame = if self.current_frame >= self.total_frames {
            1 // Wrap back to first frame
        } else {
            self.current_frame + 1
        };

        debug!(
            "Advanced from frame {} to frame {}",
            old_frame, self.current_frame
        );

        self.save_to_file(path)
            .context("Failed to save frame info after incrementing")?;

        Ok(())
    }

    /// Save the current state to a TOML file.
    ///
    /// Creates parent directories if they don't exist. The file is written
    /// in pretty-printed TOML format for easy manual editing if needed.
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path = path.as_ref();

        let toml_string =
            toml::to_string_pretty(self).context("Failed to serialize FrameInfo to TOML")?;

        // Create parent directories if they don't exist
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create parent directories for {}", path.display())
            })?;
        }

        fs::write(path, toml_string)
            .with_context(|| format!("Failed to write frame info to {}", path.display()))?;

        debug!("Saved frame info to {}", path.display());
        Ok(())
    }

    /// Load frame info from file, or create with defaults if file doesn't exist.
    ///
    /// This is the preferred way to initialize FrameInfo. If the file exists,
    /// it loads the saved progress. If not, it creates a new file with the
    /// provided defaults. This allows the bot to resume where it left off
    /// after restarts while handling first-time setup gracefully.
    pub fn load_or_create<P: AsRef<Path>>(
        path: P,
        default_total_frames: u32,
        default_current_frame: u32,
    ) -> Result<Self> {
        let path = path.as_ref();

        match fs::read_to_string(path) {
            Ok(content) => {
                debug!("Loading existing frame info from {}", path.display());
                let frame_info: FrameInfo = toml::from_str(&content)
                    .with_context(|| format!("Failed to parse TOML from {}", path.display()))?;

                // Validate loaded data
                frame_info.validate().with_context(|| {
                    format!("Invalid frame info loaded from {}", path.display())
                })?;

                info!(
                    "Loaded frame info: frame {}/{} from {}",
                    frame_info.current_frame,
                    frame_info.total_frames,
                    path.display()
                );
                Ok(frame_info)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                info!(
                    "Frame info file {} not found, creating with defaults",
                    path.display()
                );
                let frame_info = Self::new(default_total_frames, default_current_frame)
                    .context("Failed to create default FrameInfo")?;

                frame_info
                    .save_to_file(path)
                    .context("Failed to save initial frame info")?;

                info!(
                    "Created new frame info: frame {}/{}",
                    frame_info.current_frame, frame_info.total_frames
                );
                Ok(frame_info)
            }
            Err(e) => {
                Err(e).with_context(|| format!("Failed to read frame info from {}", path.display()))
            }
        }
    }

    /// Validate the current state.
    fn validate(&self) -> Result<()> {
        if self.total_frames == 0 {
            if self.current_frame != 0 {
                return Err(anyhow::anyhow!(
                    "When total_frames is 0, current_frame must also be 0"
                ));
            }
            return Ok(());
        }

        if self.current_frame == 0 {
            return Err(anyhow::anyhow!(
                "Frame numbering is 1-based, current_frame cannot be 0 when total_frames > 0"
            ));
        }

        if self.current_frame > self.total_frames {
            return Err(anyhow::anyhow!(
                "Current frame {} exceeds total frames {}",
                self.current_frame,
                self.total_frames
            ));
        }

        Ok(())
    }
}
