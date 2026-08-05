CREATE TABLE posts (
    url           TEXT PRIMARY KEY,
    dir           TEXT NOT NULL,
    title         TEXT NOT NULL,
    description   TEXT NOT NULL,
    published_at  TEXT NOT NULL,
    modified_at   TEXT NOT NULL,
    first_seen_at TEXT NOT NULL
) STRICT;

CREATE TABLE spotify_playlists (
    id         TEXT PRIMARY KEY,
    post_url   TEXT NOT NULL UNIQUE REFERENCES posts(url) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;

CREATE TABLE tracks (
    post_url            TEXT NOT NULL REFERENCES posts(url) ON DELETE CASCADE,
    post_track_number   INTEGER NOT NULL,
    title               TEXT NOT NULL,
    album_track_number  INTEGER NOT NULL,
    duration            REAL NOT NULL,
    download_url        TEXT,
    bandcamp_id         TEXT,
    spotify_id          TEXT,
    spotify_match_score REAL,
    spotify_playlist_id TEXT REFERENCES spotify_playlists(id) ON DELETE SET NULL,

    artist_name               TEXT NOT NULL,
    artist_bandcamp_id        TEXT,
    artist_bandcamp_url       TEXT,
    artist_spotify_id         TEXT,

    album_artist_name         TEXT NOT NULL,
    album_artist_bandcamp_id  TEXT,
    album_artist_bandcamp_url TEXT,
    album_artist_spotify_id   TEXT,

    album_title               TEXT NOT NULL,
    album_bandcamp_id         TEXT,
    album_bandcamp_url        TEXT,
    album_spotify_id          TEXT,

    PRIMARY KEY (post_url, post_track_number)
) STRICT;

CREATE TABLE scrapes (
    post_url   TEXT PRIMARY KEY REFERENCES posts(url) ON DELETE CASCADE,
    scraped_at TEXT NOT NULL,
    html       BLOB NOT NULL
) STRICT;
