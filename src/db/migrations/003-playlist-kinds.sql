PRAGMA defer_foreign_keys = ON;

DELETE FROM "spotify_playlists"
    WHERE "post_url" NOT IN (SELECT "url" FROM "bandcamp_posts");

DELETE FROM "bandcamp_post_tracks"
    WHERE "post_url" NOT IN (SELECT "url" FROM "bandcamp_posts");

ALTER TABLE "bandcamp_posts" ADD COLUMN "category" TEXT GENERATED ALWAYS AS (
    CASE WHEN "url" LIKE 'https://daily.bandcamp.com/%/%'
         THEN substr("url", 28, instr(substr("url", 28), '/') - 1)
    END
) VIRTUAL;

CREATE INDEX "index_bandcamp_posts_by_category" ON "bandcamp_posts" ("category");

CREATE TABLE "spotify_playlists_new" (
    "id"           TEXT NOT NULL PRIMARY KEY,
    "name"         TEXT NOT NULL,
    "kind"         TEXT NOT NULL,
    "status"       TEXT NOT NULL,
    "created_at"   TEXT NOT NULL,
    "updated_at"   TEXT NOT NULL,

    "period_start" TEXT NOT NULL,
    "period_end"   TEXT NOT NULL,

    "post_url"     TEXT REFERENCES "bandcamp_posts" ("url") ON DELETE CASCADE,

    "category"     TEXT,

    CHECK ("status" IN ('active', 'retired', 'user_deleted')),

    CHECK (
        ("kind" = 'post'
            AND "post_url"     IS NOT NULL
            AND "category"     IS NULL
            AND "period_start" = "period_end")
        OR
        ("kind" = 'category'
            AND "post_url"     IS NULL
            AND "category"     IS NOT NULL
            AND "period_start" < "period_end")
    )
) STRICT;

INSERT INTO "spotify_playlists_new" (
    "id",
    "name",
    "kind",
    "status",
    "created_at",
    "updated_at",
    "period_start",
    "period_end",
    "post_url"
) SELECT
    "p"."id",
    "p"."name",
    'post',
    CASE WHEN "p"."user_deleted" <> 0 THEN 'user_deleted' ELSE 'active' END,
    "p"."created_at",
    "p"."updated_at",
    "b"."published_at",
    "b"."published_at",
    "p"."post_url"
    FROM "spotify_playlists" AS "p"
    JOIN "bandcamp_posts" AS "b" ON "b"."url" = "p"."post_url";

DROP TABLE "spotify_playlists";

ALTER TABLE "spotify_playlists_new" RENAME TO "spotify_playlists";

CREATE UNIQUE INDEX "index_spotify_playlists_by_post_url"
    ON "spotify_playlists" ("post_url");

CREATE UNIQUE INDEX "index_spotify_playlists_by_period"
    ON "spotify_playlists" ("category", "period_start", "period_end");

CREATE INDEX "index_spotify_playlists_by_status"
    ON "spotify_playlists" ("status");

CREATE UNIQUE INDEX "index_bandcamp_post_tracks_by_track"
    ON "bandcamp_post_tracks" ("post_url", "post_track_number", "track_id");

CREATE TABLE "spotify_playlist_tracks" (
    "playlist_id"       TEXT NOT NULL REFERENCES "spotify_playlists" ("id")
                            ON DELETE CASCADE ON UPDATE CASCADE,
    "post_url"          TEXT NOT NULL,
    "post_track_number" INTEGER NOT NULL,
    "track_id"          TEXT NOT NULL,

    PRIMARY KEY ("playlist_id", "post_url", "post_track_number"),

    FOREIGN KEY ("post_url", "post_track_number", "track_id")
        REFERENCES "bandcamp_post_tracks" ("post_url", "post_track_number", "track_id")
        ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;

CREATE INDEX "index_spotify_playlist_tracks_by_track_id"
    ON "spotify_playlist_tracks" ("track_id");

CREATE INDEX "index_spotify_playlist_tracks_by_post"
    ON "spotify_playlist_tracks" ("post_url", "post_track_number");

INSERT INTO "spotify_playlist_tracks" (
    "playlist_id",
    "post_url",
    "post_track_number",
    "track_id"
) SELECT
    "spotify_playlist_id",
    "post_url",
    "post_track_number",
    "track_id"
    FROM "bandcamp_post_tracks"
    WHERE "spotify_playlist_id" IS NOT NULL;

DROP INDEX "index_bandcamp_post_tracks_by_spotify_playlist_id";

ALTER TABLE "bandcamp_post_tracks" DROP COLUMN "spotify_playlist_id";
