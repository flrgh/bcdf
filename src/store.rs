use crate::bandcamp::Scrape;
use crate::types::{Album, Artist, BlogPost, Duration, SpotifyPlaylist, Track};
use anyhow::Context;
use rusqlite::{named_params, Connection, OptionalExtension, Row, ToSql};
use std::collections::BTreeSet;
use std::path::PathBuf;

pub(crate) const DEFAULT_DATA_DIR: &str = "./data";

const DB_FILENAME: &str = "state.db";

const MIGRATIONS: &[&str] = &[include_str!("migrations/001-init.sql")];

const UPSERT_TRACK: &str = "
    INSERT INTO tracks (
        post_url,
        post_track_number,
        title,
        duration,
        download_url,
        bandcamp_id,

        artist_name,
        artist_bandcamp_id,
        artist_bandcamp_url,
        artist_spotify_id,

        album_artist_name,
        album_artist_bandcamp_id,
        album_artist_bandcamp_url,
        album_artist_spotify_id,

        album_track_number,
        album_title,
        album_bandcamp_id,
        album_bandcamp_url,
        album_spotify_id
    ) VALUES (
        :post_url,
        :post_track_number,
        :title,
        :duration,
        :download_url,
        :bandcamp_id,

        :artist_name,
        :artist_bandcamp_id,
        :artist_bandcamp_url,
        :artist_spotify_id,

        :album_artist_name,
        :album_artist_bandcamp_id,
        :album_artist_bandcamp_url,
        :album_artist_spotify_id,

        :album_track_number,
        :album_title,
        :album_bandcamp_id,
        :album_bandcamp_url,
        :album_spotify_id
    ) ON CONFLICT(post_url, post_track_number) DO UPDATE SET
        title                     = excluded.title,
        duration                  = excluded.duration,
        download_url              = excluded.download_url,
        bandcamp_id               = excluded.bandcamp_id,

        artist_name               = excluded.artist_name,
        artist_bandcamp_id        = excluded.artist_bandcamp_id,
        artist_bandcamp_url       = excluded.artist_bandcamp_url,
        artist_spotify_id         = excluded.artist_spotify_id,

        album_artist_name         = excluded.album_artist_name,
        album_artist_bandcamp_id  = excluded.album_artist_bandcamp_id,
        album_artist_bandcamp_url = excluded.album_artist_bandcamp_url,
        album_artist_spotify_id   = excluded.album_artist_spotify_id,

        album_track_number        = excluded.album_track_number,
        album_title               = excluded.album_title,
        album_bandcamp_id         = excluded.album_bandcamp_id,
        album_bandcamp_url        = excluded.album_bandcamp_url,
        album_spotify_id          = excluded.album_spotify_id";

impl TryFrom<&Row<'_>> for Track {
    type Error = rusqlite::Error;

    fn try_from(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            post_track_number: row.get("post_track_number")?,
            title: row.get("title")?,
            album_track_number: row.get("album_track_number")?,
            duration: Duration::from_secs_f64(row.get("duration")?),
            download_url: row.get("download_url")?,
            bandcamp_id: row.get("bandcamp_id")?,
            spotify_id: row.get("spotify_id")?,
            spotify_match_score: row.get("spotify_match_score")?,
            spotify_playlist_id: row.get("spotify_playlist_id")?,
            artist: Artist {
                name: row.get("artist_name")?,
                bandcamp_id: row.get("artist_bandcamp_id")?,
                bandcamp_url: row.get("artist_bandcamp_url")?,
                spotify_id: row.get("artist_spotify_id")?,
            },
            album_artist: Artist {
                name: row.get("album_artist_name")?,
                bandcamp_id: row.get("album_artist_bandcamp_id")?,
                bandcamp_url: row.get("album_artist_bandcamp_url")?,
                spotify_id: row.get("album_artist_spotify_id")?,
            },
            album: Album {
                title: row.get("album_title")?,
                bandcamp_id: row.get("album_bandcamp_id")?,
                bandcamp_url: row.get("album_bandcamp_url")?,
                spotify_id: row.get("album_spotify_id")?,
            },
        })
    }
}

fn gzip_compress(s: &str) -> anyhow::Result<Vec<u8>> {
    use flate2::Compression;
    use std::io::Write;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(s.as_bytes())?;
    Ok(gz.finish()?)
}

fn migrate(conn: &mut Connection, version: usize, sql: &str) -> anyhow::Result<()> {
    let mut tx = conn.transaction()?;
    tx.set_drop_behavior(rusqlite::DropBehavior::Rollback);

    tx.execute_batch(sql)?;
    tx.pragma_update(None, "user_version", version)?;
    tx.commit()?;

    Ok(())
}

fn init_conn(conn: &mut Connection) -> anyhow::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;

    let last_applied: usize = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .context("checking last applied user_version")?;

    for (idx, sql) in MIGRATIONS.iter().enumerate().skip(last_applied) {
        let version = idx + 1;
        migrate(conn, version, sql).with_context(|| format!("applying migration {version}"))?;
    }

    Ok(())
}

#[derive(Debug)]
pub(crate) struct Store {
    conn: Connection,
    root: PathBuf,
}

impl Store {
    pub(crate) fn open<T: AsRef<std::path::Path>>(dir: T) -> anyhow::Result<Self> {
        let root = PathBuf::from(dir.as_ref());
        std::fs::create_dir_all(&root)
            .with_context(|| format!("creating store directory {root:?}"))?;

        let path = root.join(DB_FILENAME);
        let mut conn =
            Connection::open(&path).with_context(|| format!("opening database {path:?}"))?;

        init_conn(&mut conn)?;

        Ok(Self { conn, root })
    }

    pub(crate) fn post_dir(&self, post: &BlogPost) -> PathBuf {
        self.root.join(&post.dir)
    }

    pub(crate) fn upsert_blog_post(
        &mut self,
        post: BlogPost,
        scrape: &Scrape,
    ) -> anyhow::Result<BlogPost> {
        let dir = post
            .dir
            .to_str()
            .with_context(|| format!("post directory {:?} is not utf-8", post.dir))?;

        let keep = post
            .tracks
            .iter()
            .map(|t| t.post_track_number)
            .collect::<Vec<_>>();

        let tx = self.conn.transaction()?;

        tx.execute(
            "INSERT INTO posts (
                url,
                dir,
                title,
                description,
                published_at,
                modified_at,
                first_seen_at
            ) VALUES (
                :url,
                :dir,
                :title,
                :description,
                :published_at,
                :modified_at,
                :first_seen_at
            ) ON CONFLICT(url) DO UPDATE SET
                 title        = excluded.title,
                 description  = excluded.description,
                 published_at = excluded.published_at,
                 modified_at  = excluded.modified_at",
            named_params! {
                ":url": post.url,
                ":dir": dir,
                ":title": post.title,
                ":description": post.description,
                ":published_at": post.published,
                ":modified_at": post.modified,
                ":first_seen_at": scrape.fetched_at,
            },
        )?;

        let mut params: Vec<&dyn ToSql> = Vec::with_capacity(keep.len() + 1);
        params.push(&post.url);
        params.extend(keep.iter().map(|n| n as &dyn ToSql));

        tx.execute(
            &format!(
                "DELETE FROM tracks
                  WHERE post_url = ?
                    AND post_track_number NOT IN ({})",
                vec!["?"; keep.len()].join(",")
            ),
            params.as_slice(),
        )?;

        {
            let mut stmt = tx.prepare(UPSERT_TRACK)?;
            for track in &post.tracks {
                let duration = track.duration.as_secs_f64();
                stmt.execute(named_params! {
                    ":post_url": post.url,
                    ":post_track_number": track.post_track_number,
                    ":title": track.title,
                    ":album_track_number": track.album_track_number,
                    ":duration": duration,
                    ":download_url": track.download_url,
                    ":bandcamp_id": track.bandcamp_id,
                    ":artist_name": track.artist.name,
                    ":artist_bandcamp_id": track.artist.bandcamp_id,
                    ":artist_bandcamp_url": track.artist.bandcamp_url,
                    ":artist_spotify_id": track.artist.spotify_id,
                    ":album_artist_name": track.album_artist.name,
                    ":album_artist_bandcamp_id": track.album_artist.bandcamp_id,
                    ":album_artist_bandcamp_url": track.album_artist.bandcamp_url,
                    ":album_artist_spotify_id": track.album_artist.spotify_id,
                    ":album_title": track.album.title,
                    ":album_bandcamp_id": track.album.bandcamp_id,
                    ":album_bandcamp_url": track.album.bandcamp_url,
                    ":album_spotify_id": track.album.spotify_id,
                })?;
            }
        }

        let html = gzip_compress(&scrape.html).context("compressing blog post html")?;

        tx.execute(
            "INSERT INTO scrapes (post_url, scraped_at, html)
             VALUES (:post_url, :scraped_at, :html)
             ON CONFLICT(post_url) DO UPDATE SET
                 scraped_at = excluded.scraped_at,
                 html       = excluded.html",
            named_params! {
                ":post_url": post.url,
                ":scraped_at": scrape.fetched_at,
                ":html": html,
            },
        )?;

        tx.commit()?;

        let stored = self
            .select_blog_post(&post.url)?
            .with_context(|| format!("post {} vanished after being written", post.url))?;

        let path = self.post_dir(&stored);
        std::fs::create_dir_all(&path)
            .with_context(|| format!("creating post directory {path:?}"))?;

        Ok(stored)
    }

    pub(crate) fn select_blog_post(&self, url: &str) -> anyhow::Result<Option<BlogPost>> {
        let row = self
            .conn
            .query_row(
                "SELECT dir, title, description, published_at, modified_at
                 FROM posts WHERE url = :url",
                named_params! { ":url": url },
                |row| {
                    Ok((
                        row.get::<_, String>("dir")?,
                        row.get("title")?,
                        row.get("description")?,
                        row.get("published_at")?,
                        row.get("modified_at")?,
                    ))
                },
            )
            .optional()?;

        let Some((dir, title, description, published, modified)) = row else {
            return Ok(None);
        };

        Ok(Some(BlogPost {
            url: url.to_string(),
            title,
            description,
            published,
            modified,
            dir: PathBuf::from(dir),
            tracks: self.tracks(url)?,
            spotify_playlist: self.spotify_playlist(url)?,
        }))
    }

    pub(crate) fn update_track_spotify(&self, post_url: &str, track: &Track) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE tracks
                SET spotify_id          = :spotify_id,
                    spotify_match_score = :spotify_match_score,
                    spotify_playlist_id = :spotify_playlist_id
              WHERE post_url = :post_url AND post_track_number = :post_track_number",
            named_params! {
                ":spotify_id": track.spotify_id,
                ":spotify_match_score": track.spotify_match_score,
                ":spotify_playlist_id": track.spotify_playlist_id,
                ":post_url": post_url,
                ":post_track_number": track.post_track_number,
            },
        )?;

        Ok(())
    }

    pub(crate) fn upsert_spotify_playlist(
        &self,
        post_url: &str,
        playlist: &SpotifyPlaylist,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO spotify_playlists (id, post_url, name, created_at)
             VALUES (:id, :post_url, :name, :created_at)
             ON CONFLICT(id) DO NOTHING",
            named_params! {
                ":id": playlist.id,
                ":post_url": post_url,
                ":name": playlist.name,
                ":created_at": chrono::Utc::now(),
            },
        )?;

        Ok(())
    }

    pub(crate) fn posts_with_incomplete_spotify_data(&self) -> anyhow::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT url FROM posts p
              WHERE NOT EXISTS (SELECT 1 FROM spotify_playlists s WHERE s.post_url = p.url)
                 OR EXISTS (SELECT 1 FROM tracks t
                             WHERE t.post_url = p.url
                               AND (t.spotify_id IS NULL OR t.spotify_playlist_id IS NULL))",
        )?;

        let urls = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(urls)
    }

    pub(crate) fn posts_with_incomplete_downloads(&self) -> anyhow::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT p.dir, t.* FROM posts p JOIN tracks t ON t.post_url = p.url")?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>("post_url")?,
                row.get::<_, String>("dir")?,
                Track::try_from(row)?,
            ))
        })?;

        let mut missing = BTreeSet::new();
        for row in rows {
            let (url, dir, track) = row?;
            if !self.root.join(dir).join(track.mp3_filename()).exists() {
                missing.insert(url);
            }
        }

        Ok(missing.into_iter().collect())
    }

    fn tracks(&self, post_url: &str) -> anyhow::Result<Vec<Track>> {
        let mut stmt = self.conn.prepare(
            "SELECT * FROM tracks WHERE post_url = :post_url ORDER BY post_track_number",
        )?;

        let tracks = stmt
            .query_map(named_params! { ":post_url": post_url }, |row| {
                Track::try_from(row)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(tracks)
    }

    fn spotify_playlist(&self, post_url: &str) -> anyhow::Result<Option<SpotifyPlaylist>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, name FROM spotify_playlists WHERE post_url = :post_url",
                named_params! { ":post_url": post_url },
                |row| {
                    Ok(SpotifyPlaylist {
                        id: row.get("id")?,
                        name: row.get("name")?,
                    })
                },
            )
            .optional()?)
    }
}
