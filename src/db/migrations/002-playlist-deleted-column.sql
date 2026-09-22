ALTER TABLE "spotify_playlists" ADD COLUMN "user_deleted" INTEGER NOT NULL DEFAULT 0;

CREATE INDEX "index_spotify_playlists_by_user_deleted" ON "spotify_playlists" ("user_deleted");
