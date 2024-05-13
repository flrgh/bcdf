use crate::metrics;
use id3::{frame::ExtendedText, Tag, TagLike, Version};
use std::collections::HashMap;

pub(crate) async fn tag(state: &crate::state::State) -> anyhow::Result<()> {
    for track in &state.tracks {
        let fname = state.dirname().join(track.mp3_filename());
        if !fname.exists() {
            tracing::debug!(?track, filename = ?fname, "SKIP: file does not exist");
            continue;
        }

        let mut tag = Tag::async_read_from_path(&fname).await.unwrap_or_default();

        let mut updated = false;

        if tag.title().unwrap_or("") != track.title {
            updated = true;
            tag.set_title(&track.title);
        }

        if updated || tag.artist().unwrap_or("") != track.artist.name {
            updated = true;
            tag.set_artist(&track.artist.name);
        }

        if updated || tag.album().unwrap_or("") != track.album.title {
            updated = true;
            tag.set_album(&track.album.title);
        }

        if updated || tag.album_artist().unwrap_or("") != track.album_artist.name {
            updated = true;
            tag.set_album_artist(&track.album_artist.name);
        }

        if updated || tag.track().unwrap_or(0) != track.number as u32 {
            updated = true;
            tag.set_track(track.number as u32);
        }

        let ext: HashMap<String, String> = HashMap::from_iter(
            tag.extended_texts()
                .map(|et| (et.description.clone(), et.value.clone())),
        );

        let mut set_tag = |t: &mut Tag, name: &str, value: &Option<String>| {
            let Some(value) = value else {
                return;
            };

            if ext.get(name).is_some_and(|v| *v == *value) {
                return;
            }

            updated = true;

            t.add_frame(ExtendedText {
                description: name.to_string(),
                value: value.clone(),
            });
        };

        set_tag(&mut tag, "bandcamp_track_id", &track.bandcamp_track_id);
        set_tag(&mut tag, "spotify_track_id", &track.spotify_id);
        set_tag(
            &mut tag,
            "bandcamp_playlist_track_number",
            &Some(track.bandcamp_playlist_track_number.to_string()),
        );

        set_tag(&mut tag, "bandcamp_artist_id", &track.artist.bandcamp_id);
        set_tag(&mut tag, "bandcamp_artist_url", &track.artist.bandcamp_url);
        set_tag(&mut tag, "spotify_artist_id", &track.artist.spotify_id);

        set_tag(
            &mut tag,
            "bandcamp_album_artist_id",
            &track.album_artist.bandcamp_id,
        );
        set_tag(
            &mut tag,
            "bandcamp_album_artist_url",
            &track.album_artist.bandcamp_url,
        );
        set_tag(
            &mut tag,
            "spotify_album_artist_id",
            &track.album_artist.spotify_id,
        );

        set_tag(&mut tag, "bandcamp_album_id", &track.album.bandcamp_id);
        set_tag(&mut tag, "bandcamp_album_url", &track.album.bandcamp_url);
        set_tag(&mut tag, "spotify_album_id", &track.album.spotify_id);

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
