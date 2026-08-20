use id3::{Tag, TagLike, Version, frame::ExtendedText};
use std::collections::HashMap;

use crate::db::full;
use crate::metrics;

impl crate::App {
    pub(crate) async fn tag(&self, items: &full::Post) -> anyhow::Result<()> {
        for elem in &items.tracks {
            let post_track = &elem.post_track;
            let track = &elem.track.track;
            let release = &elem.track.release;
            let artist = &elem.track.artist;

            let album_artist = &artist.name;
            let track_artist = track.credited_artist.as_ref().unwrap_or(album_artist);

            let Some(fname) = elem.post_track.filename.as_ref() else {
                tracing::debug!(?elem, "SKIP: no recorded file");
                continue;
            };

            let fname = self.path(&items.post.dir).join(fname);

            if !fname.exists() {
                tracing::debug!(?elem, filename = ?fname, "SKIP: file does not exist");
                continue;
            }

            let mut tag = Tag::async_read_from_path(&fname).await.unwrap_or_default();

            let mut updated = false;

            if updated || tag.title().unwrap_or("") != track.title {
                updated = true;
                tag.set_title(&track.title);
            }

            if updated || tag.artist().unwrap_or("") != track_artist {
                updated = true;
                tag.set_artist(track_artist);
            }

            if updated || tag.album().unwrap_or("") != release.title {
                updated = true;
                tag.set_album(&release.title);
            }

            if updated || tag.album_artist().unwrap_or("") != album_artist {
                updated = true;
                tag.set_album_artist(album_artist);
            }

            if updated || tag.track().unwrap_or(0) != track.release_track_number as u32 {
                updated = true;
                tag.set_track(track.release_track_number as u32);
            }

            let ext: HashMap<String, String> = HashMap::from_iter(
                tag.extended_texts()
                    .map(|et| (et.description.clone(), et.value.clone())),
            );

            let mut set_tag = |t: &mut Tag, name: &str, value: Option<String>| {
                let Some(value) = value else {
                    return;
                };

                if ext.get(name).is_some_and(|v| *v == value) {
                    return;
                }

                updated = true;

                t.add_frame(ExtendedText {
                    description: name.to_string(),
                    value,
                });
            };

            set_tag(&mut tag, "bandcamp_track_id", Some(track.id.clone()));
            set_tag(&mut tag, "spotify_track_id", track.spotify_id.clone());
            set_tag(
                &mut tag,
                "bandcamp_playlist_track_number",
                Some(post_track.post_track_number.to_string()),
            );

            set_tag(&mut tag, "bandcamp_artist_id", Some(artist.id.to_string()));
            set_tag(&mut tag, "bandcamp_artist_url", Some(artist.url.clone()));
            set_tag(&mut tag, "spotify_artist_id", artist.spotify_id.clone());

            set_tag(
                &mut tag,
                "bandcamp_album_artist_id",
                Some(artist.id.to_string()),
            );
            set_tag(
                &mut tag,
                "bandcamp_album_artist_url",
                Some(artist.url.clone()),
            );
            set_tag(
                &mut tag,
                "spotify_album_artist_id",
                artist.spotify_id.clone(),
            );

            set_tag(&mut tag, "bandcamp_album_id", Some(release.id.to_string()));
            set_tag(&mut tag, "bandcamp_album_url", Some(release.url.clone()));
            set_tag(&mut tag, "spotify_album_id", release.spotify_id.clone());

            if updated {
                tracing::info!(?fname, "tags updated, saving file");
                tag.write_to_path(fname, Version::Id3v24)?;
                metrics::inc(metrics::TracksWithUpdatedTags, 1);
            } else {
                tracing::debug!(?fname, "no tags were changed");
            }
        }

        Ok(())
    }
}
