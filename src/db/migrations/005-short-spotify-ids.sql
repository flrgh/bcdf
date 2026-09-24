PRAGMA defer_foreign_keys = ON;

DELETE FROM "spotify_playlist_tracks"
    WHERE "playlist_id" NOT IN (SELECT "id" FROM "spotify_playlists");

UPDATE "spotify_playlist_tracks"
    SET "playlist_id" = substr("playlist_id", 18)
    WHERE "playlist_id" LIKE 'spotify:playlist:%';

UPDATE "spotify_playlists"
    SET "id" = substr("id", 18)
    WHERE "id" LIKE 'spotify:playlist:%';

UPDATE "spotify_track_searches"
    SET "best_match_id" = substr("best_match_id", 15)
    WHERE "best_match_id" LIKE 'spotify:track:%';

UPDATE "spotify_track_searches"
    SET "prev_match_id" = substr("prev_match_id", 15)
    WHERE "prev_match_id" LIKE 'spotify:track:%';

UPDATE "spotify_track_search_candidates"
    SET "id" = substr("id", 15)
    WHERE "id" LIKE 'spotify:track:%';

UPDATE "bandcamp_tracks"
    SET "spotify_id" = substr("spotify_id", 15)
    WHERE "spotify_id" LIKE 'spotify:track:%';
