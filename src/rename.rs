use crate::store::Store;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

enum Rename {
    Done,
    Move,
    SourceMissing,
    WouldClobber,
}

impl Rename {
    fn from(from: &Path, to: &Path, files: &HashSet<PathBuf>) -> Rename {
        match (from.exists(), to.exists() || files.contains(to)) {
            (false, true) => Rename::Done,
            (true, false) => Rename::Move,
            (false, false) => Rename::SourceMissing,
            (true, true) => Rename::WouldClobber,
        }
    }
}

pub(crate) fn rename(store: &mut Store, dry_run: bool) -> anyhow::Result<()> {
    let mut state: HashSet<PathBuf> = HashSet::new();

    let (mut renamed, mut adopted, mut failed) = (0, 0, 0);

    for row in store.list_posts()? {
        let mut post = row.post;
        let recorded_dir = store.post_dir(&post);
        let mut projected_dir = recorded_dir.clone();

        let desired = post.derive_dir();
        if desired != post.dir {
            let to = store.path(&desired);

            match Rename::from(&recorded_dir, &to, &state) {
                Rename::Done => {
                    tracing::info!(from = ?recorded_dir, ?to, "post directory already moved, recording it");
                    state.insert(to.clone());
                    projected_dir = to;

                    if !dry_run {
                        match store.set_post_dir(&post.url, &desired) {
                            Ok(()) => {
                                post.dir = desired;
                                adopted += 1;
                            }
                            Err(error) => {
                                tracing::error!(
                                    ?error,
                                    url = post.url,
                                    "recording post directory failed"
                                );
                                failed += 1;
                            }
                        }
                    }
                }

                Rename::SourceMissing => {
                    tracing::warn!(from = ?recorded_dir, "recorded post directory is missing, leaving it alone");
                }

                Rename::WouldClobber => {
                    tracing::error!(from = ?recorded_dir, ?to, "post directory destination is occupied");
                    failed += 1;
                }

                Rename::Move => {
                    tracing::info!(from = ?recorded_dir, ?to, dry_run, "renaming post directory");
                    state.insert(to.clone());
                    projected_dir = to;

                    if !dry_run {
                        if let Err(error) = store.rename_post_dir(&mut post, &desired) {
                            tracing::error!(
                                ?error,
                                url = post.url,
                                "renaming post directory failed"
                            );
                            failed += 1;
                            projected_dir = recorded_dir.clone();
                        } else {
                            renamed += 1;
                        }
                    }
                }
            }
        }

        let mut tracks = std::mem::take(&mut post.tracks);

        for track in tracks.iter_mut() {
            let Some(current) = track.filename.clone() else {
                continue;
            };

            let desired = track.derive_filename();
            if current == desired {
                continue;
            }

            let from = recorded_dir.join(&current);
            let to = projected_dir.join(&desired);

            match Rename::from(&from, &to, &state) {
                Rename::Done => {
                    tracing::info!(?from, ?to, "mp3 already moved, recording it");
                    state.insert(to);

                    if !dry_run {
                        match store.set_track_filename(&post.url, track.post_track_number, &desired)
                        {
                            Ok(()) => {
                                track.filename = Some(desired);
                                adopted += 1;
                            }
                            Err(error) => {
                                tracing::error!(?error, url = post.url, "recording mp3 failed");
                                failed += 1;
                            }
                        }
                    }
                }

                Rename::SourceMissing => {
                    tracing::warn!(?from, "recorded mp3 is missing, leaving its record alone");
                }

                Rename::WouldClobber => {
                    tracing::error!(?from, ?to, "mp3 destination is occupied");
                    failed += 1;
                }

                Rename::Move => {
                    tracing::info!(?from, ?to, dry_run, "renaming mp3");
                    state.insert(to);

                    if !dry_run {
                        if let Err(error) = store.rename_track_file(&post, track, &desired) {
                            tracing::error!(?error, url = post.url, "renaming mp3 failed");
                            failed += 1;
                        } else {
                            renamed += 1;
                        }
                    }
                }
            }
        }

        post.tracks = tracks;
    }

    tracing::info!(renamed, adopted, failed, dry_run, "rename complete");

    if failed > 0 {
        anyhow::bail!("{} rename(s) failed", failed);
    }

    Ok(())
}
