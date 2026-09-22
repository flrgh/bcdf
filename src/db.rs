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
    sea_query::{ExprTrait as _, Func, OnConflict},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::bandcamp;
use crate::query::TrackQuery;
use crate::search::MatchResult;
use crate::types::{DateTime, SpotifyTrack};

mod types {
    use super::*;

    pub(crate) use string_list::StringList;
    pub(crate) mod string_list {
        use super::*;

        #[derive(
            Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult,
        )]
        pub struct StringList(pub Vec<String>);

        impl std::ops::Deref for StringList {
            type Target = Vec<String>;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl<T: ToString> FromIterator<T> for StringList {
            fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
                Self(iter.into_iter().map(|s| s.to_string()).collect())
            }
        }
    }

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

        #[sea_orm(indexed)]
        pub category: Option<String>,

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
            let in_post_playlist = entity::PlaylistTrack::find()
                .inner_join(entity::Playlist)
                .filter(col::Playlist::Kind.eq(super::playlist::Kind::Post))
                .select_only()
                .column(col::PlaylistTrack::PostUrl)
                .column(col::PlaylistTrack::PostTrackNumber)
                .into_query();

            Column::Url.in_subquery(
                entity::PostTrack::find()
                    .inner_join(entity::Track)
                    .filter(
                        Condition::any().add(col::Track::SpotifyId.is_null()).add(
                            Expr::tuple([
                                col::PostTrack::PostUrl.into_simple_expr(),
                                col::PostTrack::PostTrackNumber.into_simple_expr(),
                            ])
                            .not_in_subquery(in_post_playlist),
                        ),
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

        #[sea_orm(belongs_to, from = "track_id", to = "id")]
        pub track: BelongsTo<super::track::Entity>,

        #[sea_orm(has_many)]
        pub playlist_tracks: HasMany<super::playlist_track::Entity>,

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
                .values([overwrite_if_not_null(Filename)])
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

    #[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize)]
    #[sea_orm(
        rs_type = "String",
        db_type = "String(StringLen::None)",
        rename_all = "snake_case"
    )]
    pub(crate) enum Kind {
        Post,
        Category,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize)]
    #[sea_orm(
        rs_type = "String",
        db_type = "String(StringLen::None)",
        rename_all = "snake_case"
    )]
    pub(crate) enum Status {
        Active,

        Retired,

        // used to mark playlists that have been deleted from spotify because
        // I listened to them and chose not to keep them around
        UserDeleted,
    }

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize)]
    #[sea_orm(table_name = "spotify_playlists")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub name: String,

        pub kind: Kind,

        #[sea_orm(indexed)]
        pub status: Status,

        pub created_at: DateTime,
        pub updated_at: DateTime,

        pub period_start: DateTime,
        pub period_end: DateTime,

        #[sea_orm(indexed, unique)]
        pub post_url: Option<String>,

        #[sea_orm(indexed)]
        pub category: Option<String>,

        #[sea_orm(belongs_to, from = "post_url", to = "url")]
        pub post: BelongsTo<Option<super::post::Entity>>,

        #[sea_orm(belongs_to, from = "post_url", to = "post_url")]
        pub post_short_id: BelongsTo<Option<super::post_short_id::Entity>>,

        #[sea_orm(has_many)]
        pub tracks: HasMany<super::playlist_track::Entity>,
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
                .update_columns([
                    Name,
                    Kind,
                    PostUrl,
                    Category,
                    PeriodStart,
                    PeriodEnd,
                    UpdatedAt,
                ])
                .to_owned()
        }
    }

    #[allow(dead_code)]
    pub struct PlaylistPostTracks;

    impl Linked for PlaylistPostTracks {
        type FromEntity = Entity;
        type ToEntity = super::post_track::Entity;

        fn link(&self) -> Vec<RelationDef> {
            vec![
                Relation::PlaylistTrack.def(),
                super::playlist_track::Relation::PostTrack.def(),
            ]
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
                    .add(col::Playlist::Category.eq(q))
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

pub(crate) mod playlist_track {
    use super::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize)]
    #[sea_orm(table_name = "spotify_playlist_tracks")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false, indexed)]
        pub playlist_id: String,

        #[sea_orm(primary_key, auto_increment = false)]
        pub post_url: String,

        #[sea_orm(primary_key, auto_increment = false)]
        pub post_track_number: u32,

        #[sea_orm(indexed)]
        pub track_id: String,

        #[sea_orm(belongs_to, from = "playlist_id", to = "id")]
        pub playlist: BelongsTo<super::playlist::Entity>,

        #[sea_orm(
            belongs_to,
            from = "(post_url, post_track_number, track_id)",
            to = "(post_url, post_track_number, track_id)"
        )]
        pub post_track: BelongsTo<super::post_track::Entity>,

        #[sea_orm(belongs_to, from = "track_id", to = "id")]
        pub track: BelongsTo<super::track::Entity>,
    }

    impl ActiveModelBehavior for ActiveModel {}

    pub(crate) struct InPlaylistOrder;

    impl super::traits::ApplyTo<Select<Entity>> for InPlaylistOrder {
        fn apply_to(self, sel: Select<Entity>) -> Select<Entity> {
            sel.order_by_asc(col::Post::PublishedAt)
                .order_by_asc(Column::PostUrl)
                .order_by_asc(Column::PostTrackNumber)
        }
    }
}

pub(crate) mod search_candidate {
    use super::types::StringList;
    use super::*;

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize)]
    #[sea_orm(table_name = "spotify_track_search_candidates")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,

        pub title: String,
        pub artists: StringList,
        pub album: String,
        pub track_number: u32,
        pub duration: f64,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl super::traits::Upsert for ActiveModel {
        fn on_upsert_conflict() -> OnConflict {
            use Column::*;

            OnConflict::column(Id)
                .update_columns([Title, Artists, Album, TrackNumber, Duration])
                .to_owned()
        }
    }
}

pub(crate) mod track_search {
    use super::*;
    use crate::search::{MatchResult, Rejection};

    #[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum, Serialize)]
    #[sea_orm(
        rs_type = "String",
        db_type = "String(StringLen::None)",
        rename_all = "snake_case"
    )]
    pub(crate) enum Outcome {
        Match,
        NoResults,
        WrongVersion,
        WrongDuration,
        WrongTitle,
        WrongArtist,
    }

    impl From<&MatchResult> for Outcome {
        fn from(result: &MatchResult) -> Self {
            match result {
                MatchResult::Match(_) => Self::Match,
                MatchResult::Reject(_, Rejection::Version) => Self::WrongVersion,
                MatchResult::Reject(_, Rejection::Duration(_)) => Self::WrongDuration,
                MatchResult::Reject(_, Rejection::Title(_)) => Self::WrongTitle,
                MatchResult::Reject(_, Rejection::Artist(_)) => Self::WrongArtist,
            }
        }
    }

    #[sea_orm::model]
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize)]
    #[sea_orm(table_name = "spotify_track_searches")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub track_id: String,

        #[sea_orm(indexed)]
        pub searched_at: DateTime,
        pub search_count: u32,
        pub candidates: u32,

        #[sea_orm(indexed)]
        pub result: Outcome,

        #[sea_orm(indexed)]
        pub best_match_id: Option<String>,
        pub best_match_score: Option<f64>,

        pub prev_result: Option<Outcome>,
        pub prev_match_id: Option<String>,
        pub prev_match_score: Option<f64>,

        #[sea_orm(belongs_to, from = "track_id", to = "id")]
        pub track: BelongsTo<super::track::Entity>,

        #[sea_orm(belongs_to, from = "best_match_id", to = "id")]
        pub best_match: BelongsTo<Option<super::search_candidate::Entity>>,
    }

    impl ActiveModelBehavior for ActiveModel {}

    impl super::traits::Upsert for ActiveModel {
        fn on_upsert_conflict() -> OnConflict {
            use Column::*;

            OnConflict::column(TrackId)
                .update_columns([SearchedAt, Candidates, Result, BestMatchId, BestMatchScore])
                .values([
                    (SearchCount, Expr::col((Entity, SearchCount)).add(1)),
                    (PrevResult, Expr::col((Entity, Result))),
                    (PrevMatchId, Expr::col((Entity, BestMatchId))),
                    (PrevMatchScore, Expr::col((Entity, BestMatchScore))),
                ])
                .to_owned()
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
        artist::Entity as Artist, playlist::Entity as Playlist,
        playlist_track::Entity as PlaylistTrack, post::Entity as Post,
        post_scrape::Entity as Scrape, post_short_id::Entity as PostShortId,
        post_track::Entity as PostTrack, release::Entity as Release,
        search_candidate::Entity as SearchCandidate, track::Entity as Track,
        track_search::Entity as TrackSearch,
    };
}

pub(crate) mod col {
    pub(crate) use super::{
        artist::Column as Artist, playlist::Column as Playlist,
        playlist_track::Column as PlaylistTrack, post::Column as Post,
        post_scrape::Column as Scrape, post_short_id::Column as PostShortId,
        post_track::Column as PostTrack, release::Column as Release, track::Column as Track,
    };
}

pub(crate) mod active_model {
    #[allow(unused)]
    pub(crate) use super::{
        artist::ActiveModel as Artist, playlist::ActiveModel as Playlist,
        playlist_track::ActiveModel as PlaylistTrack, post::ActiveModel as Post,
        post_scrape::ActiveModel as Scrape, post_short_id::ActiveModel as PostShortId,
        post_track::ActiveModel as PostTrack, release::ActiveModel as Release,
        search_candidate::ActiveModel as SearchCandidate, track::ActiveModel as Track,
        track_search::ActiveModel as TrackSearch,
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
            #[sea_orm(nested)]
            pub playlist: model::Playlist,
            #[sea_orm(from_expr = "col::PlaylistTrack::TrackId.count()")]
            pub tracks: i64,
            #[sea_orm(nested)]
            pub post: Option<model::Post>,
            #[sea_orm(nested)]
            pub short_id: Option<model::PostShortId>,
        }

        impl PlaylistDetail {
            pub(crate) fn select() -> Select<entity::Playlist> {
                entity::Playlist::find()
                    .join(JoinType::LeftJoin, playlist::Relation::Post.def())
                    .join(JoinType::LeftJoin, playlist::Relation::PostShortId.def())
                    .join(JoinType::LeftJoin, playlist::Relation::PlaylistTrack.def())
                    .group_by(col::Playlist::Id)
            }

            pub(crate) fn select_unique(query: &Query) -> Select<entity::Playlist> {
                let q = query.as_str();

                Self::select().filter(
                    Condition::any()
                        .add(col::Playlist::Id.eq(q))
                        .add(col::Playlist::Name.eq(q))
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
        #[sea_orm(entity = "entity::PlaylistTrack")]
        pub struct PlaylistTrackDetail {
            #[sea_orm(nested)]
            pub post_track: model::PostTrack,
            #[sea_orm(nested)]
            pub post: model::Post,
            #[sea_orm(nested)]
            pub post_short_id: model::PostShortId,
            #[sea_orm(nested)]
            pub track: model::Track,
            #[sea_orm(nested)]
            pub release: model::Release,
            #[sea_orm(nested)]
            pub artist: model::Artist,
        }

        impl PlaylistTrackDetail {
            pub fn select(playlist_id: &str) -> Select<entity::PlaylistTrack> {
                entity::PlaylistTrack::find()
                    .join(
                        JoinType::InnerJoin,
                        playlist_track::Relation::PostTrack.def(),
                    )
                    .join(JoinType::InnerJoin, post_track::Relation::Post.def())
                    .join(JoinType::InnerJoin, post_track::Relation::PostShortId.def())
                    .join(JoinType::InnerJoin, playlist_track::Relation::Track.def())
                    .join(JoinType::InnerJoin, track::Relation::Release.def())
                    .join(JoinType::InnerJoin, release::Relation::Artist.def())
                    .filter(col::PlaylistTrack::PlaylistId.eq(playlist_id))
                    .apply(playlist_track::InPlaylistOrder)
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
        post: &model::Post,
        id: &str,
        name: &str,
    ) -> anyhow::Result<model::Playlist>;
    async fn set_playlist_status(&self, id: &str, status: playlist::Status) -> anyhow::Result<()>;
    async fn set_playlist_tracks(
        &self,
        playlist_id: &str,
        tracks: &[full::PostTrack],
    ) -> anyhow::Result<()>;
    async fn post_track_playlists(
        &self,
        post_track: &model::PostTrack,
    ) -> anyhow::Result<Vec<String>>;
    async fn record_track_search(
        &self,
        track_id: &str,
        candidates: u32,
        best: Option<(&str, &SpotifyTrack, MatchResult)>,
    ) -> anyhow::Result<()>;
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
        post: &model::Post,
        id: &str,
        name: &str,
    ) -> anyhow::Result<model::Playlist> {
        use ActiveValue::Set;

        let now = chrono::Utc::now();

        Ok(playlist::ActiveModel {
            id: Set(id.to_string()),
            name: Set(name.to_string()),
            kind: Set(playlist::Kind::Post),
            status: Set(playlist::Status::Active),
            created_at: Set(now),
            updated_at: Set(now),
            period_start: Set(post.published_at),
            period_end: Set(post.published_at),
            post_url: Set(Some(post.url.clone())),
            ..Default::default()
        }
        .upsert(self)
        .await?)
    }

    async fn set_playlist_status(&self, id: &str, status: playlist::Status) -> anyhow::Result<()> {
        entity::Playlist::update_many()
            .col_expr(col::Playlist::Status, status.into())
            .filter(col::Playlist::Id.eq(id))
            .exec(self)
            .await?;

        Ok(())
    }

    async fn set_playlist_tracks(
        &self,
        playlist_id: &str,
        tracks: &[full::PostTrack],
    ) -> anyhow::Result<()> {
        use ActiveValue::Set;

        entity::PlaylistTrack::delete_many()
            .filter(col::PlaylistTrack::PlaylistId.eq(playlist_id))
            .exec(self)
            .await?;

        let rows: Vec<active_model::PlaylistTrack> = tracks
            .iter()
            .filter(|t| t.track.track.spotify_id.is_some())
            .map(|t| active_model::PlaylistTrack {
                playlist_id: Set(playlist_id.to_string()),
                post_url: Set(t.post_track.post_url.clone()),
                post_track_number: Set(t.post_track.post_track_number),
                track_id: Set(t.post_track.track_id.clone()),
            })
            .collect();

        if !rows.is_empty() {
            entity::PlaylistTrack::insert_many(rows).exec(self).await?;
        }

        Ok(())
    }

    async fn post_track_playlists(
        &self,
        post_track: &model::PostTrack,
    ) -> anyhow::Result<Vec<String>> {
        Ok(entity::PlaylistTrack::find()
            .filter(col::PlaylistTrack::PostUrl.eq(&post_track.post_url))
            .filter(col::PlaylistTrack::PostTrackNumber.eq(post_track.post_track_number))
            .select_only()
            .column(col::PlaylistTrack::PlaylistId)
            .into_tuple()
            .all(self)
            .await?)
    }

    async fn record_track_search(
        &self,
        track_id: &str,
        candidates: u32,
        best: Option<(&str, &SpotifyTrack, MatchResult)>,
    ) -> anyhow::Result<()> {
        use ActiveValue::Set;

        let mut search = track_search::ActiveModel {
            track_id: Set(track_id.to_string()),
            searched_at: Set(chrono::Utc::now()),
            search_count: Set(1),
            candidates: Set(candidates),
            result: Set(track_search::Outcome::NoResults),
            ..Default::default()
        };

        if let Some((spotify_id, found, result)) = best {
            search_candidate::ActiveModel {
                id: Set(spotify_id.to_string()),
                title: Set(found.name.clone()),
                artists: Set(found.artists.iter().map(|artist| &artist.name).collect()),
                album: Set(found.album.name.clone()),
                track_number: Set(found.track_number),
                duration: Set(found.duration.to_std().unwrap_or_default().as_secs_f64()),
            }
            .upsert(self)
            .await?;

            search.result = Set(track_search::Outcome::from(&result));
            search.best_match_id = Set(Some(spotify_id.to_string()));
            search.best_match_score = Set(Some(result.score()));
        }

        search.upsert(self).await?;

        Ok(())
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

const MIGRATIONS: &[&str] = &[
    include_str!("db/migrations/001-init.sql"),
    include_str!("db/migrations/002-playlist-deleted-column.sql"),
    include_str!("db/migrations/003-playlist-kinds.sql"),
    include_str!("db/migrations/004-track-searches.sql"),
];

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
