use clap::Subcommand;
use std::{collections::HashSet, fs, io, os::unix::prelude::MetadataExt as _, path::PathBuf};

use crate::db::CustomQueries as _;

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

                let stat = match fs::metadata(&track_file) {
                    Ok(md) => md,
                    Err(err) => {
                        errors += 1;
                        if let io::ErrorKind::NotFound = err.kind() {
                            tracing::warn!("track file {track_file:?} does not exist");
                        } else {
                            tracing::error!("track file {track_file:?} stat failed: {err}");
                        }
                        continue;
                    }
                };

                if !stat.is_file() {
                    errors += 1;
                    tracing::warn!("track file {track_file:?} is not a regular file");
                    continue;
                }

                post_track_files.remove(&track_file);

                let size = stat.size();
                let exp = t.track.track.duration as u64 * 128 * (1024 / 8);

                let diff = size.abs_diff(exp);
                let pct = (diff as f64 / exp as f64) * 100f64;

                if pct > 20.0 {
                    let from_db = std::time::Duration::from_secs_f64(t.track.track.duration);

                    let from_file = match mp3_duration::from_path(&track_file) {
                        Ok(fd) => fd,
                        Err(e) => {
                            errors += 1;
                            tracing::error!(
                                "failed reading mp3 duration of track file {track_file:?}: {e}"
                            );
                            continue;
                        }
                    };

                    let diff = from_db.abs_diff(from_file).as_secs_f64();
                    let pct = (diff / from_db.as_secs_f64()) * 100f64;

                    if pct > 5.0 {
                        warnings += 1;

                        tracing::warn!(
                            "track file {track_file:?} is expected to be {}:{:02} but is actually {}:{:02}",
                            from_db.as_secs() / 60,
                            from_db.as_secs() % 60,
                            from_file.as_secs() / 60,
                            from_file.as_secs() % 60,
                        );
                    }
                }

                // if size > exp && pct > 30.0 {
                //     warnings += 1;
                //     let exp = exp / 1024;
                //     let size = size / 1024;
                //     let diff = diff / 1024;
                //     tracing::warn!(
                //         "track file {track_file:?} is {diff} KB ({pct:.2}%) bigger than expected ({size}KB > {exp}KB)"
                //     );
                //
                // } else if size < exp && pct > 20.0 {
                //     warnings += 1;
                //     let exp = exp / 1024;
                //     let size = size / 1024;
                //     let diff = diff / 1024;
                //
                //     tracing::warn!(
                //         "track file {track_file:?} is {diff} KB ({pct:.2}%) smaller than expected ({size}KB < {exp}KB)"
                //     );
                // }
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
