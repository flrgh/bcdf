CREATE TABLE tracks_new (
    post_url            TEXT NOT NULL REFERENCES posts(url) ON DELETE CASCADE,
    post_track_number   INTEGER NOT NULL,
    title               TEXT NOT NULL,
    album_track_number  INTEGER NOT NULL,
    duration            REAL NOT NULL,
    download_url        TEXT,
    bandcamp_id         INTEGER NOT NULL,
    spotify_id          TEXT,
    spotify_match_score REAL,
    spotify_playlist_id TEXT REFERENCES spotify_playlists(id) ON DELETE SET NULL,

    artist_name               TEXT NOT NULL,
    artist_bandcamp_id        INTEGER,
    artist_bandcamp_url       TEXT,
    artist_spotify_id         TEXT,

    album_artist_name         TEXT NOT NULL,
    album_artist_bandcamp_id  INTEGER,
    album_artist_bandcamp_url TEXT,
    album_artist_spotify_id   TEXT,

    album_title               TEXT NOT NULL,
    album_bandcamp_id         INTEGER,
    album_bandcamp_url        TEXT,
    album_spotify_id          TEXT,

    filename                  TEXT,

    PRIMARY KEY (post_url, post_track_number)
) STRICT;

INSERT INTO tracks_new (
    post_url,
    post_track_number,
    title,
    album_track_number,
    duration,
    download_url,
    bandcamp_id,
    spotify_id,
    spotify_match_score,
    spotify_playlist_id,

    artist_name,
    artist_bandcamp_id,
    artist_bandcamp_url,
    artist_spotify_id,

    album_artist_name,
    album_artist_bandcamp_id,
    album_artist_bandcamp_url,
    album_artist_spotify_id,

    album_title,
    album_bandcamp_id,
    album_bandcamp_url,
    album_spotify_id,

    filename
)
SELECT
    post_url,
    post_track_number,
    title,
    album_track_number,
    duration,
    download_url,

    -- fail if any bandcamp_id is not a valid integer
    CASE
        WHEN CAST(CAST(bandcamp_id AS INTEGER) AS TEXT) = bandcamp_id
            THEN CAST(bandcamp_id AS INTEGER)
    END,

    spotify_id,
    spotify_match_score,
    spotify_playlist_id,

    artist_name,
    CAST(artist_bandcamp_id AS INTEGER),
    artist_bandcamp_url,
    artist_spotify_id,

    album_artist_name,
    CAST(album_artist_bandcamp_id AS INTEGER),
    album_artist_bandcamp_url,
    album_artist_spotify_id,

    album_title,
    CAST(album_bandcamp_id AS INTEGER),
    album_bandcamp_url,
    album_spotify_id,

    filename
FROM tracks;

DROP TABLE tracks;

ALTER TABLE tracks_new RENAME TO tracks;
