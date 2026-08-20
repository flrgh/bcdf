CREATE TABLE "bandcamp_artists" (
    "id"         TEXT NOT NULL PRIMARY KEY,
    "url"        TEXT NOT NULL,
    "name"       TEXT NOT NULL,
    "spotify_id" TEXT
) STRICT;

CREATE INDEX "index_bandcamp_artists_by_id" ON "bandcamp_artists" ("id");

CREATE TABLE "bandcamp_releases" (
    "id"         TEXT NOT NULL PRIMARY KEY,
    "artist_id"  TEXT NOT NULL REFERENCES "bandcamp_artists" ("id") ON DELETE CASCADE,
    "type"       TEXT NOT NULL CHECK ("type" IN ('album', 'single')),
    "url"        TEXT NOT NULL,
    "title"      TEXT NOT NULL,
    "spotify_id" TEXT
) STRICT;

CREATE UNIQUE INDEX "index_bandcamp_releases_by_url" ON "bandcamp_releases" ("url");
CREATE INDEX "index_bandcamp_releases_by_artist_id" ON "bandcamp_releases" ("artist_id");

CREATE TABLE "bandcamp_tracks" (
    "id"                   TEXT NOT NULL PRIMARY KEY,
    "title"                TEXT NOT NULL,
    "duration"             REAL NOT NULL,
    "download_url"         TEXT,
    "credited_artist"      TEXT,
    "release_id"           TEXT NOT NULL REFERENCES "bandcamp_releases" ("id") ON DELETE CASCADE,
    "release_track_number" INTEGER NOT NULL,
    "spotify_id"           TEXT,
    "spotify_match_score"  REAL
) STRICT;

CREATE INDEX "index_bandcamp_tracks_by_release_id" ON "bandcamp_tracks" ("release_id");
CREATE INDEX "index_bandcamp_tracks_by_spotify_id" ON "bandcamp_tracks" ("spotify_id");

CREATE TABLE "bandcamp_posts" (
    "url"          TEXT NOT NULL PRIMARY KEY,
    "title"        TEXT NOT NULL,
    "description"  TEXT NOT NULL,
    "published_at" TEXT NOT NULL,
    "modified_at"  TEXT NOT NULL,
    "created_at"   TEXT NOT NULL,
    "updated_at"   TEXT NOT NULL,
    "dir"          TEXT NOT NULL
) STRICT;

CREATE INDEX "index_bandcamp_posts_by_dir" ON "bandcamp_posts" ("dir");

CREATE TABLE "spotify_playlists" (
    "id"         TEXT NOT NULL PRIMARY KEY,
    "name"       TEXT NOT NULL,
    "created_at" TEXT NOT NULL,
    "updated_at" TEXT NOT NULL,
    "post_url"   TEXT NOT NULL UNIQUE REFERENCES "bandcamp_posts" ("url") ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX "index_spotify_playlists_by_post_url" ON "spotify_playlists" ("post_url");

CREATE TABLE "bandcamp_post_short_ids" (
    "id"       TEXT NOT NULL PRIMARY KEY,
    "post_url" TEXT NOT NULL UNIQUE REFERENCES "bandcamp_posts" ("url") ON DELETE CASCADE
) STRICT;

CREATE UNIQUE INDEX "index_bandcamp_post_short_ids_by_id" ON "bandcamp_post_short_ids" ("id");
CREATE UNIQUE INDEX "index_bandcamp_post_short_ids_by_post_url" ON "bandcamp_post_short_ids" ("post_url");


CREATE TABLE "bandcamp_post_tracks" (
    "post_url"            TEXT NOT NULL REFERENCES "bandcamp_posts" ("url") ON DELETE CASCADE,
    "post_track_number"   INTEGER NOT NULL,
    "track_id"            TEXT NOT NULL REFERENCES "bandcamp_tracks" ("id") ON DELETE CASCADE,
    "filename"            TEXT,
    "spotify_playlist_id" TEXT,

    PRIMARY KEY ("post_url", "post_track_number")
) STRICT;

CREATE INDEX "index_bandcamp_post_tracks_by_track_id" ON "bandcamp_post_tracks" ("track_id");
CREATE INDEX "index_bandcamp_post_tracks_by_post_url" ON "bandcamp_post_tracks" ("post_url");
CREATE INDEX "index_bandcamp_post_tracks_by_spotify_playlist_id" ON "bandcamp_post_tracks" ("spotify_playlist_id");

CREATE TABLE "bandcamp_post_scrapes" (
    "url"         TEXT NOT NULL PRIMARY KEY,
    "metadata"    TEXT NOT NULL,
    "player_data" BLOB NOT NULL,
    "created_at"  TEXT NOT NULL,
    "updated_at"  TEXT NOT NULL
) STRICT;

CREATE INDEX "index_bandcamp_post_scrapes_by_url" ON "bandcamp_post_scrapes" ("url");

--


CREATE TRIGGER "bandcamp_post_short_ids_after_insert" AFTER INSERT ON "bandcamp_posts" BEGIN
DELETE FROM "bandcamp_post_short_ids";

INSERT INTO "bandcamp_post_short_ids" (
    "id",
    "post_url"
) SELECT
    printf('%s.%02d',
           strftime('%Y-%m-%d', "published_at"),
           row_number() OVER (PARTITION BY date("published_at") ORDER BY "published_at", "url")
    ),
    "url"
    FROM "bandcamp_posts";
END;

CREATE TRIGGER "bandcamp_post_short_ids_after_update" AFTER UPDATE ON "bandcamp_posts" BEGIN
DELETE FROM "bandcamp_post_short_ids";

INSERT INTO "bandcamp_post_short_ids" (
    "id",
    "post_url"
) SELECT
    printf('%s.%02d',
           strftime('%Y-%m-%d', "published_at"),
           row_number() OVER (PARTITION BY date("published_at") ORDER BY "published_at", "url")
    ),
    "url"
    FROM "bandcamp_posts";
END;

CREATE TRIGGER "bandcamp_post_short_ids_after_delete" AFTER DELETE ON "bandcamp_posts" BEGIN
DELETE FROM "bandcamp_post_short_ids";

INSERT INTO "bandcamp_post_short_ids" (
    "id",
    "post_url"
) SELECT
    printf('%s.%02d',
           strftime('%Y-%m-%d', "published_at"),
           row_number() OVER (PARTITION BY date("published_at") ORDER BY "published_at", "url")
    ),
    "url"
    FROM "bandcamp_posts";
END;

