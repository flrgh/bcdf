use anyhow::Context;
pub(crate) use sea_orm::{
    ActiveModelTrait,
    ActiveValue,
    ColumnTrait,
    //ColumnTypeTrait as _,
    Condition,
    ConnectionTrait,
    Database,
    DatabaseConnection as Db,
    //DatabaseTransaction as Transaction,
    DerivePartialModel,
    //EntityLoaderTrait as _,
    EntityTrait,
    //Insert as _,
    IntoActiveModel,
    IntoSimpleExpr,
    JoinType,
    //ModelTrait as _,
    Order,
    QueryFilter,
    QueryOrder,
    QueryResult,
    QuerySelect,
    QueryTrait,
    RelationTrait,
    Select,
    SelectModel,
    Selector,
    //SelectorTrait as _,
    Statement,
    //StatementBuilder as _,
    TransactionSession,
    TransactionTrait,
    //TryIntoModel as _,
    entity::prelude::*,
    sea_query::{
        Func,
        //ExprTrait as _,
        OnConflict,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::bandcamp;
use crate::query::TrackQuery;
use crate::types::DateTime;

mod types {
    use super::*;

    pub(crate) use map::Map;
    pub(crate) mod map {
        use super::*;

        #[derive(
            Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult,
        )]
        pub struct Map(pub HashMap<String, String>);

        impl std::ops::Deref for Map {
            type Target = HashMap<String, String>;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl std::ops::DerefMut for Map {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl From<HashMap<String, String>> for Map {
            fn from(value: HashMap<String, String>) -> Self {
                Self(value)
            }
        }

        impl From<&HashMap<String, String>> for Map {
            fn from(value: &HashMap<String, String>) -> Self {
                Self(value.clone())
            }
        }
    }
}

pub(crate) use traits::*;
pub(crate) mod traits {
    use super::*;

    pub(crate) use upsert::*;
    pub(crate) mod upsert {
        use super::*;

        pub(crate) trait Upsert: ActiveModelTrait + Sized {
            fn on_upsert_conflict() -> OnConflict;

            async fn upsert<C>(self, db: &C) -> Result<<Self::Entity as EntityTrait>::Model, DbErr>
            where
                C: ConnectionTrait,
                <Self::Entity as EntityTrait>::Model: IntoActiveModel<Self>,
            {
                <Self::Entity as EntityTrait>::insert(self)
                    .on_conflict(Self::on_upsert_conflict())
                    .exec_with_returning(db)
                    .await
            }
        }
    }

    pub(crate) use apply::*;
    pub(crate) mod apply {
        use super::*;

        pub trait ApplyTo<T> {
            fn apply_to(self, target: T) -> T;
        }

        impl<F, T: Sized> ApplyTo<T> for F
        where
            F: FnOnce(T) -> T,
        {
            fn apply_to(self, target: T) -> T {
                self(target)
            }
        }

        impl<F, T: Sized> ApplyTo<T> for Option<F>
        where
            F: ApplyTo<T>,
        {
            fn apply_to(self, target: T) -> T {
                match self {
                    Some(app) => app.apply_to(target),
                    None => target,
                }
            }
        }

        pub trait Apply: Sized {
            fn apply<T>(self, apply: T) -> Self
            where
                T: ApplyTo<Self>,
            {
                apply.apply_to(self)
            }
        }

        impl<T: QueryTrait> Apply for T {}
    }

    pub(crate) use order::*;
    pub(crate) mod order {
        use super::*;

        pub fn reverse(order: Order, reverse: bool) -> Order {
            match (reverse, order) {
                (true, Order::Asc) => Order::Desc,
                (true, Order::Desc) => Order::Asc,
                (_, order) => order,
            }
        }
    }

    pub(crate) use limit::*;
    pub(crate) mod limit {
        use super::*;

        pub trait LimitIf: Sized {
            fn limit_if<L: Into<Option<u64>>>(self, limit: L) -> Self;
        }

        impl<T: QueryTrait + QuerySelect> LimitIf for T {
            fn limit_if<L: Into<Option<u64>>>(self, limit: L) -> Self {
                match limit.into() {
                    Some(limit) => self.limit(limit),
                    None => self,
                }
            }
        }
    }

    pub(crate) use spotify_ids::*;
    pub(crate) mod spotify_ids {
        use rspotify::model::{LibraryId, PlaylistId};

        pub(crate) trait SpotifyPlaylistId {
            fn playlist_id(&self) -> PlaylistId<'_>;

            fn library_id(&self) -> LibraryId<'_> {
                LibraryId::Playlist(self.playlist_id())
            }
        }

        impl SpotifyPlaylistId for PlaylistId<'_> {
            fn playlist_id(&self) -> PlaylistId<'_> {
                self.as_ref()
            }
        }
    }
}

pub(crate) mod post {
    use super::*;

    /// A bandcamp daily post, its tracks, and the Spotify playlist they belong to.
    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize)]
    #[sea_orm(table_name = "bandcamp_posts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub url: String,
        pub title: String,
        pub description: String,

        /// the post's published timestamp, sourced from the post metadata
        pub published_at: DateTime,

        /// the post's modified timestamp, sourced from the post metadata
        pub modified_at: DateTime,

        pub created_at: DateTime,
        pub updated_at: DateTime,

        #[sea_orm(indexed)]
        pub dir: String,

        #[sea_orm(has_many)]
        pub tracks: HasMany<super::post_track::Entity>,

        #[sea_orm(has_one)]
        pub spotify_playlist: HasOne<super::playlist::Entity>,

        #[sea_orm(has_one)]
        pub short_id: HasOne<super::post_short_id::Entity>,

        #[sea_orm(has_one)]
        pub scrape: HasOne<super::post_scrape::Entity>,
    }

    pub type SelectUrl = sea_orm::Selector<sea_orm::SelectGetableValue<String, Column>>;

    pub trait Urls {
        fn urls(self) -> SelectUrl;
    }

    impl Urls for Select<Entity> {
        fn urls(self) -> SelectUrl {
            self.select_only().column(Column::Url).into_values()
        }
    }

    impl Entity {
        pub(crate) fn missing_spotify_tracks() -> Expr {
            Column::Url.in_subquery(
                entity::PostTrack::find()
                    .inner_join(entity::Track)
                    .filter(
                        Condition::any()
                            .add(col::Track::SpotifyId.is_null())
                            .add(col::PostTrack::SpotifyPlaylistId.is_null()),
                    )
                    .select_only()
                    .column(col::PostTrack::PostUrl)
                    .into_query(),
            )
        }

        pub(crate) fn missing_track_downloads() -> Expr {
            Column::Url.in_subquery(
                entity::PostTrack::find()
                    .filter(col::PostTrack::Filename.is_null())
                    .select_only()
                    .column(col::PostTrack::PostUrl)
                    .into_query(),
            )
        }
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl super::traits::Upsert for ActiveModel {
        fn on_upsert_conflict() -> OnConflict {
            use Column::*;

            OnConflict::column(Url)
                .update_columns([Title, Description, PublishedAt, ModifiedAt, UpdatedAt])
                .to_owned()
        }
    }

    impl Model {
        pub(crate) fn derive_post_dir(published: &DateTime, url: &str) -> String {
            let path = url
                .strip_suffix('/')
                .unwrap_or(url)
                .rsplit('/')
                .next()
                .expect("bandamp post url has at least one path component");

            assert!(!path.is_empty(), "empty post slug for {url}");
            format!("{}-{}", published.format("%Y-%m-%d"), path).replace('/', "_")
        }

        pub(crate) fn derive_dir(&self) -> String {
            Self::derive_post_dir(&self.published_at, &self.url)
        }

        pub(crate) fn playlist_name(&self) -> String {
            format!(
                "bcdf {}: {}",
                self.published_at.format("%y%m%d"),
                self.title,
            )
        }

        pub(crate) async fn tracks<C>(&self, db: &C) -> anyhow::Result<Vec<model::PostTrack>>
        where
            C: ConnectionTrait,
        {
            Ok(self.find_related(entity::PostTrack).all(db).await?)
        }
    }

    impl super::traits::ApplyTo<Select<Entity>> for crate::query::Query {
        fn apply_to(self, sel: Select<Entity>) -> Select<Entity> {
            use Column::*;

            let q = self.as_str();
            let url = bandcamp::post_url(q);

            sel.filter(
                Condition::any()
                    .add(Url.eq(url.as_ref()))
                    .add(Dir.eq(q))
                    .add(Url.contains(q))
                    .add(Dir.contains(q)),
            )
        }
    }
}

pub(crate) mod post_scrape {
    use super::*;

    // naming `Meta` as to not collide with the exported `Metadata` Column
    pub(crate) use super::types::Map as Meta;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize)]
    #[sea_orm(table_name = "bandcamp_post_scrapes")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub url: String,

        pub metadata: Meta,

        #[sea_orm(column_type = "Blob")]
        pub player_data: Vec<u8>,

        pub created_at: DateTime,
        pub updated_at: DateTime,

        #[sea_orm(belongs_to, from = "url", to = "url")]
        pub post: BelongsTo<super::post::Entity>,
    }

    impl ActiveModelBehavior for ActiveModel {}

    use crate::compress::decompress_json as gz_to_json;

    impl Model {
        pub fn player_json(&self) -> anyhow::Result<Vec<serde_json::Value>> {
            gz_to_json(&self.player_data)
        }
    }

    impl super::traits::Upsert for ActiveModel {
        fn on_upsert_conflict() -> OnConflict {
            use Column::*;

            OnConflict::column(Url)
                .update_columns([Metadata, PlayerData, UpdatedAt])
                .to_owned()
        }
    }

    impl super::traits::ApplyTo<Select<Entity>> for crate::query::Query {
        fn apply_to(self, sel: Select<Entity>) -> Select<Entity> {
            use Column::*;

            let q = self.as_str();
            let url = bandcamp::post_url(q);

            sel.filter(
                Condition::any()
                    .add(Url.eq(url.as_ref()))
                    .add(Url.contains(q)),
            )
        }
    }

    impl Entity {
        pub(crate) async fn get_player_json(
            db: &Db,
            url: &str,
        ) -> anyhow::Result<Vec<serde_json::Value>> {
            Entity::find_by_id(url.to_string())
                .one(db)
                .await?
                .with_context(|| format!("no scrape recorded for {url}"))?
                .player_json()
        }
    }
}

pub(crate) mod post_short_id {
    use super::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize)]
    #[sea_orm(table_name = "bandcamp_post_short_ids")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,

        #[sea_orm(indexed, unique)]
        pub post_url: String,

        #[sea_orm(belongs_to, from = "post_url", to = "url")]
        pub post: BelongsTo<super::post::Entity>,
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub(crate) mod post_track {
    use super::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize)]
    #[sea_orm(table_name = "bandcamp_post_tracks")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false, indexed)]
        pub post_url: String,

        #[sea_orm(primary_key, auto_increment = false)]
        pub post_track_number: u32,

        #[sea_orm(indexed)]
        pub track_id: String,

        pub filename: Option<String>,

        #[sea_orm(indexed)]
        pub spotify_playlist_id: Option<String>,

        #[sea_orm(belongs_to, from = "track_id", to = "id")]
        pub track: BelongsTo<super::track::Entity>,

        #[sea_orm(belongs_to, from = "spotify_playlist_id", to = "id")]
        pub spotify_playlist: BelongsTo<Option<super::playlist::Entity>>,

        #[sea_orm(belongs_to, from = "post_url", to = "url")]
        pub post: BelongsTo<super::post::Entity>,

        #[sea_orm(belongs_to, from = "post_url", to = "post_url")]
        pub post_short_id: BelongsTo<super::post_short_id::Entity>,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl super::traits::Upsert for ActiveModel {
        fn on_upsert_conflict() -> OnConflict {
            use Column::*;

            fn overwrite_if_not_null(c: Column) -> (Column, Expr) {
                (
                    c,
                    Func::coalesce([Expr::col(("excluded", c)), Expr::col((Entity, c))]).into(),
                )
            }

            OnConflict::columns([PostUrl, PostTrackNumber])
                .update_column(TrackId)
                .values([
                    overwrite_if_not_null(Filename),
                    overwrite_if_not_null(SpotifyPlaylistId),
                ])
                .to_owned()
        }
    }

    impl super::traits::ApplyTo<Select<Entity>> for TrackQuery {
        fn apply_to(self, sel: Select<Entity>) -> Select<Entity> {
            match self {
                TrackQuery::Any(query) => {
                    let q = query.as_str();

                    sel.filter(
                        Condition::any()
                            .add(col::Track::Id.eq(q))
                            .add(col::Track::SpotifyId.eq(q))
                            .add(col::PostShortId::Id.eq(q))
                            .add(col::Track::Title.contains(q))
                            .add(col::Artist::Name.contains(q))
                            .add(col::Release::Title.contains(q))
                            .add(col::Post::Title.contains(q))
                            .add(col::Post::Url.contains(q))
                            .add(col::Post::Dir.contains(q))
                            .add(col::PostTrack::Filename.contains(q)),
                    )
                }

                TrackQuery::InPost { post, number } => {
                    let q = post.as_str();
                    let url = bandcamp::post_url(q);

                    sel.filter(
                        Condition::all()
                            .add(col::PostTrack::PostTrackNumber.eq(number))
                            .add(
                                Condition::any()
                                    .add(col::Post::Url.eq(url.as_ref()))
                                    .add(col::Post::Dir.eq(q))
                                    .add(col::PostShortId::Id.eq(q))
                                    .add(col::Post::Dir.contains(q))
                                    .add(col::Post::Title.contains(q)),
                            ),
                    )
                }
            }
        }
    }
}

pub(crate) mod track {
    use super::*;
    use rspotify::{model::TrackId, prelude::PlayableId};

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize)]
    #[sea_orm(table_name = "bandcamp_tracks")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,

        pub title: String,
        pub duration: f64,
        pub download_url: Option<String>,

        // set when the track artist does not match the album artist
        pub credited_artist: Option<String>,

        #[sea_orm(indexed)]
        pub release_id: String,
        pub release_track_number: u16,

        #[sea_orm(indexed)]
        pub spotify_id: Option<String>,
        pub spotify_match_score: Option<f64>,

        #[sea_orm(belongs_to, from = "release_id", to = "id")]
        pub release: BelongsTo<super::release::Entity>,

        #[sea_orm(has_many)]
        pub post_tracks: HasMany<super::post_track::Entity>,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl super::traits::Upsert for ActiveModel {
        fn on_upsert_conflict() -> OnConflict {
            use Column::*;

            OnConflict::column(Id)
                .update_columns([
                    Title,
                    Duration,
                    DownloadUrl,
                    CreditedArtist,
                    ReleaseId,
                    ReleaseTrackNumber,
                ])
                .to_owned()
        }
    }

    #[allow(dead_code)]
    pub struct TrackArtist;

    impl Linked for TrackArtist {
        type FromEntity = Entity;
        type ToEntity = super::artist::Entity;

        fn link(&self) -> Vec<RelationDef> {
            vec![
                Relation::Release.def(),
                super::release::Relation::Artist.def(),
            ]
        }
    }

    impl Model {
        pub fn spotify_track_id(&self) -> Option<TrackId<'_>> {
            self.spotify_id
                .as_ref()
                .map(|sid| TrackId::from_id_or_uri(sid).expect("we never save invalid track ids"))
        }

        pub fn spotify_playable_id(&self) -> Option<PlayableId<'_>> {
            self.spotify_track_id().map(PlayableId::Track)
        }
    }
}

pub(crate) mod artist {
    use super::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize)]
    #[sea_orm(table_name = "bandcamp_artists")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub url: String,
        pub name: String,
        pub spotify_id: Option<String>,

        #[sea_orm(has_many)]
        pub releases: HasMany<super::release::Entity>,
    }

    impl ActiveModelBehavior for ActiveModel {}

    #[allow(dead_code)]
    pub struct ArtistTracks;

    impl Linked for ArtistTracks {
        type FromEntity = Entity;
        type ToEntity = super::track::Entity;

        fn link(&self) -> Vec<RelationDef> {
            vec![
                Relation::Release.def(),
                super::release::Relation::Track.def(),
            ]
        }
    }

    impl super::traits::Upsert for ActiveModel {
        fn on_upsert_conflict() -> OnConflict {
            use Column::*;

            OnConflict::column(Id)
                .update_columns([Url, Name])
                .to_owned()
        }
    }
}

pub(crate) mod release {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize)]
    #[sea_orm(
        rs_type = "String",
        db_type = "String(StringLen::None)",
        rename_all = "snake_case"
    )]
    pub(crate) enum Type {
        Album,
        Single,
    }

    impl Type {
        pub(crate) fn try_from_char(c: char) -> anyhow::Result<Self> {
            match c {
                'a' => Ok(Self::Album),
                't' => Ok(Self::Single),
                _ => anyhow::bail!("unknown parent_tralbum_type char value: '{c}'"),
            }
        }
    }

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize)]
    #[sea_orm(table_name = "bandcamp_releases")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,

        #[sea_orm(column_name = "type")]
        pub release_type: Type,

        #[sea_orm(indexed, unique)]
        pub url: String,
        pub title: String,
        pub spotify_id: Option<String>,

        #[sea_orm(indexed)]
        pub artist_id: String,

        #[sea_orm(belongs_to, from = "artist_id", to = "id")]
        pub artist: BelongsTo<super::artist::Entity>,

        #[sea_orm(has_many)]
        pub tracks: HasMany<super::track::Entity>,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl super::traits::Upsert for ActiveModel {
        fn on_upsert_conflict() -> OnConflict {
            use Column::*;

            OnConflict::column(Id)
                .update_columns([ReleaseType, Url, Title, ArtistId])
                .to_owned()
        }
    }
}

pub(crate) mod playlist {
    use super::*;
    use rspotify::model::PlaylistId;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize)]
    #[sea_orm(table_name = "spotify_playlists")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub name: String,

        pub created_at: DateTime,
        pub updated_at: DateTime,

        #[sea_orm(indexed, unique)]
        pub post_url: String,

        #[sea_orm(belongs_to, from = "post_url", to = "url")]
        pub post: BelongsTo<super::post::Entity>,

        #[sea_orm(belongs_to, from = "post_url", to = "post_url")]
        pub post_short_id: BelongsTo<super::post_short_id::Entity>,
        #[sea_orm(has_many)]
        pub tracks: HasMany<super::post_track::Entity>,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn spotify_playlist_id(&self) -> PlaylistId<'_> {
            PlaylistId::from_id_or_uri(&self.id).expect("we never save invalid playlist ids")
        }
    }

    impl SpotifyPlaylistId for Model {
        fn playlist_id(&self) -> PlaylistId<'_> {
            self.spotify_playlist_id()
        }
    }

    impl super::traits::Upsert for ActiveModel {
        fn on_upsert_conflict() -> OnConflict {
            use Column::*;

            OnConflict::column(Id)
                .update_columns([Name, PostUrl, UpdatedAt])
                .to_owned()
        }
    }

    #[allow(dead_code)]
    pub struct PlaylistPostTracks;

    impl Linked for PlaylistPostTracks {
        type FromEntity = Entity;
        type ToEntity = super::post_track::Entity;

        fn link(&self) -> Vec<RelationDef> {
            vec![Relation::Post.def(), super::post::Relation::PostTrack.def()]
        }
    }

    impl super::traits::ApplyTo<Select<Entity>> for crate::query::Query {
        fn apply_to(self, sel: Select<Entity>) -> Select<Entity> {
            let q = self.as_str();
            let url = bandcamp::post_url(q);

            sel.filter(
                Condition::any()
                    .add(col::Playlist::Id.eq(q))
                    .add(col::Playlist::Name.eq(q))
                    .add(col::Post::Url.eq(url.as_ref()))
                    .add(col::Post::Dir.eq(q))
                    .add(col::PostShortId::Id.eq(q))
                    .add(col::Playlist::Id.contains(q))
                    .add(col::Playlist::Name.contains(q))
                    .add(col::Post::Url.contains(q))
                    .add(col::Post::Dir.contains(q))
                    .add(col::PostShortId::Id.contains(q)),
            )
        }
    }
}

pub(crate) mod model {
    pub use super::{
        artist::Model as Artist, playlist::Model as Playlist, post::Model as Post,
        post_scrape::Model as Scrape, post_short_id::Model as PostShortId,
        post_track::Model as PostTrack, release::Model as Release, track::Model as Track,
    };
}

pub(crate) mod entity {
    #[allow(unused)]
    pub use super::{
        artist::Entity as Artist, playlist::Entity as Playlist, post::Entity as Post,
        post_scrape::Entity as Scrape, post_short_id::Entity as PostShortId,
        post_track::Entity as PostTrack, release::Entity as Release, track::Entity as Track,
    };
}

pub(crate) mod col {
    pub(crate) use super::{
        artist::Column as Artist, playlist::Column as Playlist, post::Column as Post,
        post_scrape::Column as Scrape, post_short_id::Column as PostShortId,
        post_track::Column as PostTrack, release::Column as Release, track::Column as Track,
    };
}

pub(crate) mod active_model {
    #[allow(unused)]
    pub(crate) use super::{
        artist::ActiveModel as Artist, playlist::ActiveModel as Playlist,
        post::ActiveModel as Post, post_scrape::ActiveModel as Scrape,
        post_short_id::ActiveModel as PostShortId, post_track::ActiveModel as PostTrack,
        release::ActiveModel as Release, track::ActiveModel as Track,
    };
}

pub(crate) use views::*;
pub(crate) mod views {
    use crate::query::{Query, TrackQuery};
    use rspotify::model::PlayableId;

    use super::*;

    #[derive(Debug, DerivePartialModel)]
    pub(crate) struct TrackAll {
        #[sea_orm(nested)]
        pub(crate) track: model::Track,

        #[sea_orm(nested)]
        pub(crate) release: model::Release,

        #[sea_orm(nested)]
        pub(crate) artist: model::Artist,
    }

    #[derive(Debug, DerivePartialModel)]
    pub(crate) struct PostTrackAll {
        #[sea_orm(nested)]
        pub(crate) post_track: model::PostTrack,

        #[sea_orm(nested)]
        pub(crate) post: model::Post,

        #[sea_orm(nested)]
        pub(crate) post_short_id: model::PostShortId,

        #[sea_orm(nested)]
        pub(crate) track: TrackAll,
    }

    impl PostTrackAll {
        pub fn spotify_playable_id(&self) -> Option<PlayableId<'_>> {
            self.track.track.spotify_playable_id()
        }

        pub(crate) fn derive_filename(&self) -> String {
            let artist = self
                .track
                .track
                .credited_artist
                .as_ref()
                .unwrap_or(&self.track.artist.name);

            format!(
                "{:02} - {} - {}.mp3",
                self.post_track.post_track_number, artist, self.track.track.title
            )
            .replace('/', "_")
            .replace('\n', "_")
        }
    }

    #[derive(Debug, DerivePartialModel)]
    #[sea_orm(entity = "entity::Post")]
    pub(crate) struct PostItems {
        #[sea_orm(nested)]
        pub(crate) post: model::Post,

        #[sea_orm(nested)]
        pub(crate) short_id: model::PostShortId,

        #[sea_orm(nested)]
        pub(crate) playlist: Option<model::Playlist>,

        #[sea_orm(skip)]
        pub(crate) tracks: Vec<PostTrackAll>,
    }

    impl PostItems {
        pub(crate) fn select() -> Select<entity::Post> {
            entity::Post::find()
                .inner_join(entity::PostShortId)
                .left_join(entity::Playlist)
        }

        pub(crate) async fn get<C: ConnectionTrait>(
            db: &C,
            url_or_short_id: &str,
        ) -> anyhow::Result<Option<Self>> {
            Self::load(
                db,
                Condition::any()
                    .add(col::Post::Url.eq(bandcamp::post_url(url_or_short_id).as_ref()))
                    .add(col::PostShortId::Id.eq(url_or_short_id)),
            )
            .await
        }

        pub(crate) async fn find_unique(db: &Db, query: &Query) -> anyhow::Result<Option<Self>> {
            let q = query.as_str();

            Self::load(
                db,
                Condition::any()
                    .add(col::Post::Url.eq(bandcamp::post_url(q).as_ref()))
                    .add(col::Post::Dir.eq(q))
                    .add(col::PostShortId::Id.eq(q))
                    .add(col::Playlist::Id.eq(q)),
            )
            .await
        }

        async fn load<C: ConnectionTrait>(
            db: &C,
            filter: Condition,
        ) -> anyhow::Result<Option<Self>> {
            let Some(mut post) = Self::select()
                .filter(filter)
                .into_partial_model::<Self>()
                .one(db)
                .await?
            else {
                return Ok(None);
            };

            post.tracks = post.load_tracks().all(db).await?;

            Ok(Some(post))
        }

        pub(crate) fn load_tracks(&self) -> Selector<SelectModel<PostTrackAll>> {
            PostTrackAll::select()
                .filter(col::PostTrack::PostUrl.eq(&self.post.url))
                .order_by_asc(col::PostTrack::PostTrackNumber)
                .into_partial_model()
        }

        pub(crate) fn downloaded_count(&self) -> usize {
            self.tracks
                .iter()
                .filter(|t| t.post_track.filename.is_some())
                .count()
        }

        pub(crate) fn spotify_count(&self) -> usize {
            self.tracks
                .iter()
                .filter(|t| t.track.track.spotify_id.is_some())
                .count()
        }
    }

    /// Every scrape column except the compressed player blob.
    #[derive(Debug, Serialize, DerivePartialModel)]
    #[sea_orm(entity = "entity::Scrape")]
    pub(crate) struct ScrapeInfo {
        pub(crate) url: String,
        pub(crate) metadata: post_scrape::Meta,
        pub(crate) created_at: DateTime,
        pub(crate) updated_at: DateTime,
    }

    #[derive(Debug, DerivePartialModel)]
    #[sea_orm(entity = "entity::Post")]
    pub(crate) struct PostSummary {
        #[sea_orm(nested)]
        pub(crate) post: model::Post,

        #[sea_orm(nested)]
        pub(crate) short_id: model::PostShortId,

        #[sea_orm(nested)]
        pub(crate) playlist: Option<model::Playlist>,

        #[sea_orm(from_expr = "col::PostTrack::TrackId.count()")]
        pub(crate) tracks: i64,

        #[sea_orm(from_expr = "col::PostTrack::Filename.count()")]
        pub(crate) downloaded: i64,

        #[sea_orm(from_expr = "col::Track::SpotifyId.count()")]
        pub(crate) spotify: i64,
    }

    impl PostSummary {
        pub(crate) fn select() -> Select<entity::Post> {
            entity::Post::find()
                .inner_join(entity::PostShortId)
                .left_join(entity::Playlist)
                .left_join(entity::PostTrack)
                .join(JoinType::LeftJoin, post_track::Relation::Track.def())
                .group_by(col::Post::Url)
        }
    }

    impl PostTrackAll {
        pub(crate) fn select() -> Select<entity::PostTrack> {
            entity::PostTrack::find()
                .inner_join(entity::Post)
                .inner_join(entity::PostShortId)
                .inner_join(entity::Track)
                .join(JoinType::InnerJoin, track::Relation::Release.def())
                .join(JoinType::InnerJoin, release::Relation::Artist.def())
        }

        pub(crate) async fn find_unique(db: &Db, query: &TrackQuery) -> anyhow::Result<Self> {
            let mut rows: Vec<Self> = Self::select()
                .apply(query.clone())
                .into_partial_model()
                .all(db)
                .await?;

            match rows.len() {
                0 => anyhow::bail!("no track matched {query}"),
                1 => Ok(rows.remove(0)),
                n => anyhow::bail!("{n} tracks matched {query}"),
            }
        }
    }

    pub(crate) use playlist_detail::*;
    mod playlist_detail {
        use super::*;
        use crate::query::Query;

        #[derive(Debug, DerivePartialModel)]
        #[sea_orm(entity = "entity::Playlist")]
        pub(crate) struct PlaylistDetail {
            pub id: String,
            pub name: String,
            #[sea_orm(from_expr = "col::PostTrack::TrackId.count()")]
            pub tracks: i64,
            #[sea_orm(nested)]
            pub post: model::Post,
            #[sea_orm(nested)]
            pub short_id: model::PostShortId,
        }

        impl PlaylistDetail {
            pub(crate) fn select() -> Select<entity::Playlist> {
                entity::Playlist::find()
                    .join(JoinType::InnerJoin, playlist::Relation::Post.def())
                    .join(JoinType::InnerJoin, playlist::Relation::PostShortId.def())
                    .join(JoinType::LeftJoin, playlist::Relation::PostTrack.def())
                    .group_by(col::Playlist::Id)
            }

            pub(crate) fn select_unique(query: &Query) -> Select<entity::Playlist> {
                let q = query.as_str();

                Self::select().filter(
                    Condition::any()
                        .add(col::Playlist::Id.eq(q))
                        .add(col::Post::Url.eq(crate::bandcamp::post_url(q).as_ref()))
                        .add(col::Post::Dir.eq(q))
                        .add(col::PostShortId::Id.eq(q)),
                )
            }
        }
    }

    pub(crate) use playlist_track_detail::*;
    mod playlist_track_detail {
        use super::*;

        #[derive(Debug, DerivePartialModel)]
        #[sea_orm(entity = "entity::PostTrack")]
        pub struct PlaylistTrackDetail {
            pub post_track_number: u32,
            pub filename: Option<String>,
            #[sea_orm(nested)]
            pub track: model::Track,
            #[sea_orm(nested)]
            pub release: model::Release,
            #[sea_orm(nested)]
            pub artist: model::Artist,
        }

        impl PlaylistTrackDetail {
            pub fn select(playlist_id: &str) -> Select<entity::PostTrack> {
                entity::PostTrack::find()
                    .join(JoinType::InnerJoin, post_track::Relation::Track.def())
                    .join(JoinType::InnerJoin, track::Relation::Release.def())
                    .join(JoinType::InnerJoin, release::Relation::Artist.def())
                    .filter(col::PostTrack::SpotifyPlaylistId.eq(playlist_id))
                    .order_by_asc(col::PostTrack::PostTrackNumber)
            }
        }
    }
}

pub(crate) mod full {
    pub(crate) use super::views::{
        PostItems as Post, PostTrackAll as PostTrack, TrackAll as Track,
    };
}

pub(crate) trait CustomQueries {
    async fn get_post(&self, url_or_short_id: &str) -> anyhow::Result<views::PostItems>;
    async fn all_posts(&self) -> anyhow::Result<Vec<views::PostItems>>;
    async fn update_post_track(
        &self,
        post_track: &model::PostTrack,
    ) -> anyhow::Result<model::PostTrack>;
    async fn update_track(&self, track: &model::Track) -> anyhow::Result<model::Track>;
    async fn upsert_playlist(
        &self,
        post_url: &str,
        id: &str,
        name: &str,
    ) -> anyhow::Result<model::Playlist>;
}

impl<T: ConnectionTrait> CustomQueries for T {
    async fn get_post(&self, url_or_short_id: &str) -> anyhow::Result<views::PostItems> {
        let Some(post) = views::PostItems::get(self, url_or_short_id).await? else {
            anyhow::bail!("no post found for {url_or_short_id}");
        };
        Ok(post)
    }

    async fn all_posts(&self) -> anyhow::Result<Vec<views::PostItems>> {
        let mut posts: Vec<views::PostItems> =
            PostItems::select().into_partial_model().all(self).await?;

        // FIXME: N+1 select
        for p in posts.iter_mut() {
            p.tracks = p.load_tracks().all(self).await?;
        }

        Ok(posts)
    }

    async fn update_track(&self, track: &model::Track) -> anyhow::Result<model::Track> {
        Ok(track
            .clone()
            .into_active_model()
            .reset_all()
            .update(self)
            .await?)
    }

    async fn upsert_playlist(
        &self,
        post_url: &str,
        id: &str,
        name: &str,
    ) -> anyhow::Result<model::Playlist> {
        use ActiveValue::Set;

        let now = chrono::Utc::now();

        Ok(playlist::ActiveModel {
            id: Set(id.to_string()),
            post_url: Set(post_url.to_string()),
            name: Set(name.to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .upsert(self)
        .await?)
    }

    async fn update_post_track(
        &self,
        post_track: &model::PostTrack,
    ) -> anyhow::Result<model::PostTrack> {
        Ok(post_track
            .clone()
            .into_active_model()
            .reset_all()
            .update(self)
            .await?)
    }
}

const MIGRATIONS: &[&str] = &[include_str!("db/migrations/001-init.sql")];

async fn migrate(conn: &mut Db, version: usize, sql: &str) -> anyhow::Result<()> {
    let tx = conn.begin().await?;
    tx.execute_unprepared(sql).await?;
    tx.execute_unprepared(&format!("PRAGMA user_version = {version}"))
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn init_conn(db: &mut Db) -> anyhow::Result<()> {
    db.execute_unprepared("PRAGMA journal_mode = WAL").await?;
    db.execute_unprepared("PRAGMA foreign_keys = true").await?;

    let version: Option<QueryResult> = db
        .query_one_raw(Statement::from_string(
            db.get_database_backend(),
            "PRAGMA user_version;",
        ))
        .await?;

    let last_applied: u32 = version
        .and_then(|q| q.try_get("", "user_version").ok())
        .unwrap_or(0);

    for (idx, sql) in MIGRATIONS.iter().enumerate().skip(last_applied as usize) {
        let version = idx + 1;
        migrate(db, version, sql)
            .await
            .with_context(|| format!("applying migration {version}"))?;
    }

    Ok(())
}

pub(crate) async fn open<T: AsRef<std::path::Path>>(path: T) -> anyhow::Result<Db> {
    let path = path.as_ref();

    let url = if path == std::path::Path::new(":memory:") {
        "sqlite::memory:".to_string()
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating db directory {parent:?}"))?;
        }

        format!("sqlite://{}", path.display())
    };

    let mut opts = sea_orm::ConnectOptions::new(&url);
    opts.map_sqlx_sqlite_opts(|opts| opts.create_if_missing(true));

    let mut db = Database::connect(opts).await?;

    init_conn(&mut db).await?;

    Ok(db)
}
