CREATE TABLE "spotify_track_search_candidates" (
    "id"                TEXT NOT NULL PRIMARY KEY,

    "title"             TEXT NOT NULL,
    "artists"           TEXT NOT NULL,
    "album"             TEXT NOT NULL,
    "track_number"      INTEGER NOT NULL,
    "duration"          REAL NOT NULL,

    CHECK (json_valid("artists") AND json_type("artists") = 'array')
) STRICT;

CREATE TABLE "spotify_track_searches" (
    "track_id"          TEXT NOT NULL PRIMARY KEY
                            REFERENCES "bandcamp_tracks" ("id") ON DELETE CASCADE,

    "searched_at"       TEXT NOT NULL,
    "search_count"      INTEGER NOT NULL DEFAULT 1,

    "candidates"        INTEGER NOT NULL,

    "result"            TEXT NOT NULL,
    "best_match_id"     TEXT REFERENCES "spotify_track_search_candidates" ("id"),

    "best_match_score"  REAL,

    "prev_result"       TEXT,
    "prev_match_id"     TEXT REFERENCES "spotify_track_search_candidates" ("id"),
    "prev_match_score"  REAL,

    CHECK ("search_count" > 0),

    CHECK ("result" IN (
        'match',
        'no_results',
        'wrong_version',
        'wrong_duration',
        'wrong_title',
        'wrong_artist'
    )),

    CHECK ("prev_result" IS NULL OR "prev_result" IN (
        'match',
        'no_results',
        'wrong_version',
        'wrong_duration',
        'wrong_title',
        'wrong_artist'
    )),

    CHECK (("result" = 'no_results') = ("candidates" = 0)),

    CHECK (("search_count" = 1) = ("prev_result" IS NULL)),

    CHECK (
        ("result" =  'no_results'
            AND "best_match_id"    IS     NULL
            AND "best_match_score" IS     NULL)
        OR
        ("result" <> 'no_results'
            AND "best_match_id"    IS NOT NULL
            AND "best_match_score" IS NOT NULL)
    ),

    CHECK (
        ("prev_result" IS NULL
            AND "prev_match_id"    IS NULL
            AND "prev_match_score" IS NULL)
        OR
        ("prev_result" =  'no_results'
            AND "prev_match_id"    IS     NULL
            AND "prev_match_score" IS     NULL)
        OR
        ("prev_result" <> 'no_results'
            AND "prev_match_id"    IS NOT NULL
            AND "prev_match_score" IS NOT NULL)
    )
) STRICT;

CREATE INDEX "index_spotify_track_searches_by_searched_at"
    ON "spotify_track_searches" ("searched_at");

CREATE INDEX "index_spotify_track_searches_by_result"
    ON "spotify_track_searches" ("result");

CREATE INDEX "index_spotify_track_searches_by_best_match_id"
    ON "spotify_track_searches" ("best_match_id");
