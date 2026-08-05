use crate::bandcamp::Scrape;
use crate::types::{Album, Artist, BlogPost, Duration, SpotifyPlaylist, Track};
use anyhow::Context;
use rusqlite::{named_params, Connection, OptionalExtension, Row, ToSql};
use std::collections::{BTreeSet, HashMap};
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

const POST_COLUMNS: &str = "url, dir, title, description, published_at, modified_at";

impl TryFrom<&Row<'_>> for BlogPost {
    type Error = rusqlite::Error;

    fn try_from(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            url: row.get("url")?,
            title: row.get("title")?,
            description: row.get("description")?,
            published: row.get("published_at")?,
            modified: row.get("modified_at")?,
            dir: PathBuf::from(row.get::<_, String>("dir")?),
            tracks: Vec::new(),
            spotify_playlist: None,
        })
    }
}

impl TryFrom<&Row<'_>> for SpotifyPlaylist {
    type Error = rusqlite::Error;

    fn try_from(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
        })
    }
}

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

    pub(crate) fn upsert_post(
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
            .get_post(&post.url)?
            .with_context(|| format!("post {} vanished after being written", post.url))?;

        let path = self.post_dir(&stored);
        std::fs::create_dir_all(&path)
            .with_context(|| format!("creating post directory {path:?}"))?;

        Ok(stored)
    }

    pub(crate) fn get_post(&self, url: &str) -> anyhow::Result<Option<BlogPost>> {
        let post: Option<BlogPost> = self
            .conn
            .query_row(
                &format!("SELECT {POST_COLUMNS} FROM posts WHERE url = :url"),
                named_params! { ":url": url },
                |row| row.try_into(),
            )
            .optional()?;

        let Some(mut post) = post else {
            return Ok(None);
        };

        post.tracks = self.tracks(url)?;
        post.spotify_playlist = self.spotify_playlist(url)?;

        Ok(Some(post))
    }

    pub(crate) fn list_posts(&self) -> anyhow::Result<Vec<BlogPost>> {
        let mut tracks = self.tracks_by_post()?;
        let mut playlists = self.spotify_playlists_by_post()?;

        let mut stmt = self.conn.prepare(&format!(
            "SELECT {POST_COLUMNS} FROM posts ORDER BY published_at"
        ))?;

        let posts = stmt
            .query_map([], |row| row.try_into())?
            .collect::<rusqlite::Result<Vec<BlogPost>>>()?;

        Ok(posts
            .into_iter()
            .map(|mut post| {
                post.tracks = tracks.remove(&post.url).unwrap_or_default();
                post.spotify_playlist = playlists.remove(&post.url);
                post
            })
            .collect())
    }

    fn tracks_by_post(&self) -> anyhow::Result<HashMap<String, Vec<Track>>> {
        let mut stmt = self
            .conn
            .prepare("SELECT * FROM tracks ORDER BY post_url, post_track_number")?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>("post_url")?, row.try_into()?))
        })?;

        let mut by_post: HashMap<String, Vec<Track>> = HashMap::new();
        for row in rows {
            let (post_url, track) = row?;
            by_post.entry(post_url).or_default().push(track);
        }

        Ok(by_post)
    }

    fn spotify_playlists_by_post(&self) -> anyhow::Result<HashMap<String, SpotifyPlaylist>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, post_url, name FROM spotify_playlists")?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>("post_url")?, row.try_into()?))
        })?;

        let mut by_post = HashMap::new();
        for row in rows {
            let (post_url, playlist) = row?;
            by_post.insert(post_url, playlist);
        }

        Ok(by_post)
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
                row.try_into()?,
            ))
        })?;

        let mut missing = BTreeSet::new();
        for row in rows {
            let (url, dir, track): (String, String, Track) = row?;
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
                row.try_into()
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
                |row| row.try_into(),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bandcamp::Scrape;

    fn store(root: &tempfile::TempDir) -> Store {
        let mut conn = Connection::open_in_memory().expect("in-memory database should open");
        init_conn(&mut conn).expect("migrations should apply to an empty database");

        Store {
            conn,
            root: root.path().to_path_buf(),
        }
    }

    fn scrape() -> Scrape {
        Scrape {
            html: "<html></html>".to_string(),
            fetched_at: chrono::Utc::now(),
        }
    }

    fn post(url: &str, title: &str, published: &str, tracks: Vec<Track>) -> BlogPost {
        let published = published.parse().expect("timestamp should be rfc3339");
        BlogPost {
            url: url.to_string(),
            title: title.to_string(),
            description: format!("{title} description"),
            published,
            modified: published,
            dir: PathBuf::from(title),
            tracks,
            spotify_playlist: None,
        }
    }

    fn track(number: usize, title: &str) -> Track {
        let mut track = Track::new(title, "artist", "album");
        track.post_track_number = number;
        track.album_track_number = number;
        track
    }

    #[test]
    fn list_posts_agrees_with_get_post() {
        let root = tempfile::tempdir().unwrap();
        let mut store = store(&root);

        let a = post(
            "https://example.test/a",
            "A",
            "2024-01-01T00:00:00Z",
            vec![track(1, "one"), track(2, "two")],
        );
        let b = post(
            "https://example.test/b",
            "B",
            "2024-02-01T00:00:00Z",
            vec![],
        );

        store.upsert_post(a, &scrape()).unwrap();
        store.upsert_post(b, &scrape()).unwrap();

        let expected: Vec<BlogPost> = ["https://example.test/a", "https://example.test/b"]
            .iter()
            .map(|url| store.get_post(url).unwrap().expect("post was just written"))
            .collect();

        assert_eq!(store.list_posts().unwrap(), expected);
    }

    #[test]
    fn list_posts_orders_by_published_date() {
        let root = tempfile::tempdir().unwrap();
        let mut store = store(&root);

        for (url, title, published) in [
            ("https://example.test/late", "Late", "2024-03-01T00:00:00Z"),
            (
                "https://example.test/early",
                "Early",
                "2024-01-01T00:00:00Z",
            ),
        ] {
            store
                .upsert_post(post(url, title, published, vec![]), &scrape())
                .unwrap();
        }

        let titles: Vec<String> = store
            .list_posts()
            .unwrap()
            .into_iter()
            .map(|p| p.title)
            .collect();

        assert_eq!(titles, ["Early", "Late"]);
    }
}
