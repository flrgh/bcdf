use clap::Subcommand;
use std::time::Duration;
use std::{collections::HashSet, fs, io, os::unix::prelude::MetadataExt as _, path::PathBuf};

use crate::db::{CustomQueries as _, views};

#[derive(Debug)]
pub(crate) enum TrackError {
    NotFound,
    NotAFile,
    Mismatch,
    Other,
}

pub(crate) fn check_track(
    track: &views::PostTrackAll,
    track_file: &PathBuf,
) -> Result<(), TrackError> {
    let stat = match fs::metadata(track_file) {
        Ok(md) => md,
        Err(err) => {
            if let io::ErrorKind::NotFound = err.kind() {
                tracing::error!("track file {track_file:?} does not exist");
                return Err(TrackError::NotFound);
            } else {
                tracing::error!("failed to stat() file {track_file:?}: {err}");
                return Err(TrackError::Other);
            }
        }
    };

    if !stat.is_file() {
        tracing::error!("track file {track_file:?} is not a regular file");
        return Err(TrackError::NotAFile);
    }

    let size = stat.size();
    let exp = track.track.track.duration as u64 * 128 * (1024 / 8);

    let diff = size.abs_diff(exp);
    let pct = (diff as f64 / exp as f64) * 100f64;

    if pct > 1.0 || true {
        let from_db = Duration::from_secs_f64(track.track.track.duration);

        let from_file = match mp3_duration::from_path(track_file) {
            Ok(fd) => fd,
            Err(e) => {
                tracing::error!("failed reading mp3 duration of track file {track_file:?}: {e}");
                return Err(TrackError::Other);
            }
        };

        let diff = from_db.abs_diff(from_file).as_secs_f64();

        if diff > 5.0 {
            tracing::warn!(
                "track file {track_file:?} is expected to be {}:{:02}s but is actually {}:{:02}s",
                from_db.as_secs() / 60,
                from_db.as_secs() % 60,
                from_file.as_secs() / 60,
                from_file.as_secs() % 60,
            );

            return Err(TrackError::Mismatch);
        }
    }

    Ok(())
}

impl crate::App {
    pub(crate) async fn check(&self) -> anyhow::Result<()> {
        let (mut posts, mut tracks) = (0, 0);
        let mut errors = 0;
        let mut warnings = 0;

        let mut post_track_files: HashSet<PathBuf> = HashSet::new();

        for row in self.db.all_posts().await? {
            posts += 1;
            tracing::debug!("post: {}", &row.post.dir);

            let post_dir = self.path(&row.post.dir);

            post_track_files.extend(
                fs::read_dir(&post_dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|entry| entry.path()),
            );

            for t in row.tracks.iter() {
                tracks += 1;

                let Some(fname) = &t.post_track.filename else {
                    continue;
                };

                let track_file = post_dir.join(fname);
                if let Err(e) = check_track(t, &track_file) {
                    match e {
                        TrackError::NotFound | TrackError::NotAFile | TrackError::Other => {
                            errors += 1;
                            continue;
                        }
                        TrackError::Mismatch => {
                            warnings += 1;
                        }
                    }
                }

                post_track_files.remove(&track_file);
            }

            let mut leftover: Vec<PathBuf> = post_track_files.drain().collect();
            leftover.sort();
            for entry in leftover {
                tracing::warn!("orphaned file in post directory: {entry:?}");
                warnings += 1;
            }
        }

        tracing::info!("checked {posts} posts, {tracks} tracks");

        if warnings > 0 {
            tracing::warn!("{warnings} warnings");
        } else {
            tracing::info!("{warnings} warnings");
        }

        if errors > 0 {
            anyhow::bail!("{errors} errors");
        } else {
            tracing::info!("{errors} errors");
        }

        Ok(())
    }
}

/// Manage downloaded mp3 files
#[derive(clap::Args, Debug)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    pub(crate) async fn exec(self, app: &crate::App) -> anyhow::Result<()> {
        match self.command {
            Command::Download => {
                for mut row in app.db.all_posts().await? {
                    app.download(&mut row).await?;
                }
            }
            Command::Tag => {
                for row in app.db.all_posts().await? {
                    app.tag(&row).await?;
                }
            }
            Command::Rename { dry_run } => {
                app.rename(dry_run).await?;
            }
            Command::Check => {
                app.check().await?;
            }
        };

        Ok(())
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Download any mp3 files missing from the data directory
    Download,

    /// Rewrite the id3 tags of every mp3 file recorded as downloaded
    Tag,

    /// Rename post directories and mp3 files to match the current naming
    /// scheme, updating the database to match
    Rename {
        /// Report what would be renamed without touching anything
        #[arg(short = 'n', long)]
        dry_run: bool,
    },

    /// Check for missing/broken/unexpected files in the data directory
    Check,
}
