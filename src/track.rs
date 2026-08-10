use crate::list::Filters;
use crate::query::{normalize, Matchable, TrackQuery, DATE_FORMAT};
use crate::store::Store;
use crate::types::{BlogPost, Format, Track};
use clap::Subcommand;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::io::{self, Write};

/// View and manage tracks
#[derive(clap::Args, Debug)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    pub(crate) async fn exec(self, store: &Store) -> anyhow::Result<()> {
        match self.command {
            Command::Ls(ls) => ls.exec(store),
            Command::Show(show) => show.exec(store),
            Command::Match(match_) => match_.exec(store).await,
        }
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List tracks
    Ls(List),

    /// Show a single track
    Show(Show),

    /// Attempt to match (or re-match with -f|--force) a Bandcamp track to a Spotify track
    Match(Match),
}

#[derive(clap::Args, Debug)]
pub(crate) struct List {
    /// Select tracks matching this text, or <post>/<number>
    #[arg(value_name = "QUERY")]
    query: Option<TrackQuery>,

    #[arg(long, value_enum, default_value = "post")]
    sort: Sort,

    /// Reverse the sort order
    #[arg(short, long)]
    reverse: bool,

    #[command(flatten)]
    filters: Filters,

    #[arg(short, long, value_enum, default_value = "table")]
    output: Format,
}

impl List {
    fn exec(self, store: &Store) -> anyhow::Result<()> {
        let posts = store.list_posts()?;

        let mut rows = match &self.query {
            Some(query) => query.filter(&posts),
            None => TrackRow::all(&posts),
        };

        rows.retain(|row| self.filters.published_at(&row.post.published));

        rows.sort_by(|a, b| {
            if self.reverse {
                self.sort.compare(a, b).reverse()
            } else {
                self.sort.compare(a, b)
            }
        });

        self.filters.limit_results(&mut rows);

        let mut out = std::io::stdout().lock();

        match self.output {
            Format::Table => {
                writeln!(out, "POST\t{CHILD_HEADER}")?;
                for row in rows {
                    write!(out, "{}\t", row.post.dir)?;
                    row.write_child_row(&mut out)?;
                }
            }
            Format::Json => {
                let values: Vec<Value> = rows.iter().map(|row| row.json()).collect();
                serde_json::to_writer_pretty(&mut out, &values)?;
                writeln!(out)?;
            }
        };

        Ok(())
    }
}

#[derive(clap::Args, Debug)]
pub(crate) struct Show {
    /// Select the track matching this text, or <post>/<number>
    #[arg(value_name = "QUERY")]
    query: TrackQuery,

    #[arg(short, long, value_enum, default_value = "table")]
    output: Format,
}

impl Show {
    fn exec(self, store: &Store) -> anyhow::Result<()> {
        let posts = store.list_posts()?;
        let row = self.query.filter_unique(&posts)?;

        let mut out = std::io::stdout().lock();
        match self.output {
            Format::Table => {
                let track = row.track;

                writeln!(out, "post\t{}", row.post.dir)?;
                writeln!(out, "number\t{}", track.post_track_number)?;
                writeln!(out, "title\t{}", track.title)?;
                writeln!(out, "artist\t{}", track.artist.name)?;
                writeln!(out, "album_artist\t{}", track.album_artist.name)?;
                writeln!(out, "album\t{}", track.album.title)?;

                let secs = track.duration.as_secs();
                writeln!(out, "duration\t{}:{:02}", secs / 60, secs % 60)?;

                writeln!(out, "bandcamp_id\t{}", track.bandcamp_id)?;
                writeln!(out, "spotify_id\t{}", opt_string(&track.spotify_id))?;
                writeln!(
                    out,
                    "spotify_playlist_id\t{}",
                    opt_string(&track.spotify_playlist_id)
                )?;
                writeln!(out, "file\t{}", opt_string(&track.filename))?;
                writeln!(out, "download_url\t{}", opt_string(&track.download_url))?;
            }
            Format::Json => {
                serde_json::to_writer_pretty(&mut out, &row.json())?;
                writeln!(out)?;
            }
        };

        Ok(())
    }
}

#[derive(clap::Args, Debug)]
pub(crate) struct Match {
    /// Select the track matching this text, or <post>/<number>
    #[arg(value_name = "QUERY")]
    query: TrackQuery,

    #[arg(short, long, default_value_t = false)]
    force: bool,

    #[arg(short = 'n', long, default_value_t = false)]
    dry_run: bool,
}

impl Match {
    async fn exec(self, store: &Store) -> anyhow::Result<()> {
        let posts = store.list_posts()?;
        let row = self.query.filter_unique(&posts)?;

        fn fmt_score(s: &Option<f64>) -> String {
            match s {
                Some(v) => v.to_string(),
                None => "unknown".to_string(),
            }
        }

        let old_id = &row.track.spotify_id;
        let old_score = &row.track.spotify_match_score;
        if let (Some(id), false) = (old_id, self.force) {
            println!(
                "unchanged: already matched ('{} - {}' => {}, score: {})",
                row.track.artist.name,
                row.track.title,
                id,
                fmt_score(old_score)
            );
            return Ok(());
        };

        let spotify = crate::spotify::connect().await?;
        let mut track = row.track.clone();
        track.spotify_id = None;
        track.spotify_match_score = None;
        spotify.search(&mut track).await?;

        let update = match (old_id, &track.spotify_id) {
            (Some(old), Some(new)) => {
                let new_score = &track.spotify_match_score;

                if old == new {
                    dbg!(&old_score, &new_score);
                    println!("unchanged: {old} is still the best spotify track match");
                    if old_score != new_score {
                        println!(
                            "match score will be updated from {} -> {}",
                            fmt_score(old_score),
                            fmt_score(new_score)
                        );
                        true
                    } else {
                        false
                    }
                } else {
                    println!(
                        "changed: {old} (score: {}) => {new} (score: {})",
                        fmt_score(old_score),
                        fmt_score(new_score)
                    );
                    true
                }
            }
            (None, None) => {
                println!("no new spotify match found :(");
                false
            }
            (None, Some(new)) => {
                println!(
                    "new match: {new} (score: {})",
                    fmt_score(&track.spotify_match_score)
                );
                true
            }
            (Some(old), None) => {
                println!("changed: {old} (score: {}) will be removed. This might mean the track no longer exists on Spotify or that updated search criteria determined it to no longer be a suitable match", fmt_score(old_score));
                true
            }
        };

        if update {
            if self.dry_run {
                println!("dry run: no database changes made");
            } else {
                store.update_track_spotify(&row.post.url, &track)?;
            }
        }

        Ok(())
    }
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
enum Sort {
    /// Sort by newest post, then track number
    Post,
    /// Sort by track title, case-insensitive
    Title,
    /// Sort by track artist name, case-insensitive
    Artist,
    /// Sort by track album name, case-insensitive
    Album,
}

impl Sort {
    fn compare(self, a: &TrackRow, b: &TrackRow) -> Ordering {
        let by_field = match self {
            Self::Post => b.post.published.cmp(&a.post.published),
            Self::Title => normalize(&a.track.title).cmp(&normalize(&b.track.title)),
            Self::Artist => normalize(&a.track.artist.name).cmp(&normalize(&b.track.artist.name)),
            Self::Album => normalize(&a.track.album.title).cmp(&normalize(&b.track.album.title)),
        };

        by_field
            .then_with(|| a.post.dir.cmp(&b.post.dir))
            .then_with(|| a.track.post_track_number.cmp(&b.track.post_track_number))
    }
}

pub(crate) const CHILD_HEADER: &str = "#\tARTIST\tALBUM\tDUR\tDL\tSPOTIFY\tTITLE";

/// A Track plus its parent BlogPost
#[derive(Clone, Copy)]
pub(crate) struct TrackRow<'a> {
    post: &'a BlogPost,
    track: &'a Track,
}

impl<'a> TrackRow<'a> {
    pub(crate) fn new(post: &'a BlogPost, track: &'a Track) -> Self {
        Self { post, track }
    }

    pub(crate) fn all(posts: &'a [BlogPost]) -> Vec<Self> {
        posts
            .iter()
            .flat_map(|post| post.tracks.iter().map(move |track| Self::new(post, track)))
            .collect()
    }

    pub(crate) fn write_child_row(&self, out: &mut dyn Write) -> io::Result<()> {
        let track = self.track;

        write!(out, "{}\t", track.post_track_number)?;
        write!(out, "{}\t", track.artist.name)?;
        write!(out, "{}\t", track.album.title)?;

        let secs = track.duration.as_secs();
        write!(out, "{}:{:02}\t", secs / 60, secs % 60)?;

        write!(out, "{}\t", if track.filename.is_some() { "y" } else { "" })?;
        write!(out, "{}\t", track.spotify_id.as_deref().unwrap_or_default())?;
        writeln!(out, "{}", track.title)
    }

    pub(crate) fn child_json(&self) -> Value {
        let track = self.track;

        json!({
            "title": track.title,
            "artist": track.artist,
            "album_artist": track.album_artist,
            "album": track.album,
            "duration_secs": track.duration.as_secs_f64(),
            "post_track_number": track.post_track_number,
            "album_track_number": track.album_track_number,
            "bandcamp_id": track.bandcamp_id,
            "download_url": track.download_url,
            "spotify": track.spotify_id.as_ref().map(|id| json!({
                "id": id,
                "match_score": track.spotify_match_score,
                "playlist_id": track.spotify_playlist_id,
            })),
            // relative to the data directory, and what the database records
            // rather than what is on disk
            "file": track.filename.as_ref().map(|name| json!({
                "name": name,
                "path": format!("{}/{}", self.post.dir, name),
            })),
        })
    }

    fn json(&self) -> Value {
        let mut value = self.child_json();
        value["post"] = self.post.json_summary();
        value
    }
}

fn opt_string(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or_default()
}

impl Matchable for TrackRow<'_> {
    fn search_fields(&self) -> Vec<Cow<'_, str>> {
        let track = self.track;

        let mut fields = vec![
            Cow::from(track.title.as_str()),
            Cow::from(track.artist.name.as_str()),
            Cow::from(track.album_artist.name.as_str()),
            Cow::from(track.album.title.as_str()),
            Cow::from(track.bandcamp_id.to_string()),
            Cow::from(self.post.dir.as_str()),
            Cow::from(self.post.url.as_str()),
            Cow::from(self.post.title.as_str()),
            Cow::from(self.post.published.format(DATE_FORMAT).to_string()),
        ];

        let optional = [
            &track.filename,
            &track.spotify_id,
            &track.spotify_playlist_id,
        ];

        for value in optional.into_iter().flatten() {
            fields.push(Cow::from(value.as_str()));
        }

        fields
    }

    fn identity_fields(&self) -> Vec<Cow<'_, str>> {
        let mut fields = vec![Cow::from(self.track.bandcamp_id.to_string())];

        if let Some(spotify_id) = &self.track.spotify_id {
            fields.push(Cow::from(spotify_id.as_str()));
        }

        fields
    }
}
