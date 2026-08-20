use anyhow::Context;

use crate::db::{
    ActiveModelTrait as _, ActiveValue, CustomQueries as _, IntoActiveModel as _,
    TransactionTrait as _, full, model,
};

impl crate::App {
    async fn rename_post_dir(&self, post: &mut model::Post, dry_run: bool) -> anyhow::Result<bool> {
        let desired = post.derive_dir();

        if desired == post.dir {
            return Ok(false);
        }

        let from = self.path(&post.dir);
        let to = self.path(&desired);

        if !from.exists() {
            anyhow::bail!("recorded post path does not exist");
        } else if !from.is_dir() {
            anyhow::bail!("recorded post path is not a directory");
        } else if to.exists() {
            anyhow::bail!("desired post path already exists");
        }

        tracing::info!(?from, ?to, dry_run, "renaming post directory");

        if !dry_run {
            let tx = self.db.begin().await?;

            let updated = {
                let mut active = post.clone().into_active_model();
                active.dir = ActiveValue::Set(desired);
                active.update(&tx).await?
            };

            std::fs::rename(&from, &to).with_context(|| format!("renaming {from:?} to {to:?}"))?;

            tx.commit().await?;

            *post = updated;
        }

        Ok(true)
    }

    async fn rename_track_file(
        &self,
        post: &model::Post,
        track: &mut full::PostTrack,
        dry_run: bool,
    ) -> anyhow::Result<bool> {
        let Some(current) = track.post_track.filename.as_ref() else {
            return Ok(false);
        };

        let desired = track.derive_filename();
        if *current == desired {
            return Ok(false);
        }

        let post_dir = self.path(&post.dir);

        let from = post_dir.join(current);
        let to = post_dir.join(&desired);

        tracing::info!(?from, ?to, dry_run, "renaming track mp3 filename");

        if !from.exists() {
            anyhow::bail!("recorded track filename is missing");
        } else if !from.is_file() {
            anyhow::bail!("recorded track filename is not a regular file");
        } else if to.exists() {
            anyhow::bail!("desired track filename already exists");
        }

        if !dry_run {
            let tx = self.db.begin().await?;

            let updated = {
                let mut active = track.post_track.clone().into_active_model();
                active.filename = ActiveValue::Set(Some(desired));
                active.update(&tx).await?
            };

            std::fs::rename(&from, &to).with_context(|| format!("renaming {from:?} to {to:?}"))?;

            tx.commit().await?;

            track.post_track = updated;
        }

        Ok(true)
    }

    pub(crate) async fn rename(&self, dry_run: bool) -> anyhow::Result<()> {
        let mut renamed = 0;
        let mut failed = 0;

        for mut items in self.db.all_posts().await? {
            match self.rename_post_dir(&mut items.post, dry_run).await {
                Ok(true) => renamed += 1,
                Ok(false) => {}
                Err(error) => {
                    tracing::error!(
                        ?error,
                        url = items.post.url,
                        "renaming post directory failed"
                    );
                    failed += 1;
                    continue;
                }
            }

            for track in items.tracks.iter_mut() {
                match self.rename_track_file(&items.post, track, dry_run).await {
                    Ok(true) => renamed += 1,
                    Ok(false) => {}
                    Err(error) => {
                        tracing::error!(
                            ?error,
                            url = track.post_track.post_url,
                            "renaming track mp3 filename failed"
                        );
                        failed += 1;
                        continue;
                    }
                }
            }
        }

        tracing::info!(renamed, failed, dry_run, "rename complete");

        if failed > 0 {
            anyhow::bail!("{} rename(s) failed", failed);
        }

        Ok(())
    }
}
