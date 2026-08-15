use crate::types::DateTime;
use anyhow::anyhow;
use chrono::{Local, Months, NaiveDate, TimeDelta, Utc};
use comfy_table::presets::UTF8_FULL_CONDENSED;
use comfy_table::{CellAlignment, ColumnConstraint, ContentArrangement, Table};
use interim::{Dialect, Interval};
use std::str::FromStr;

/// `ls` options
#[derive(clap::Args, Debug)]
pub(crate) struct Filters {
    /// Limit the total number of items returned
    #[arg(short = 'n', long)]
    limit: Option<usize>,

    /// Limit to posts published >= TIMESPEC
    #[arg(long, value_name = "TIMESPEC")]
    since: Option<Timespec>,

    /// Limit to posts published <= TIMESPEC
    #[arg(long, value_name = "TIMESPEC")]
    until: Option<Timespec>,
}

impl Filters {
    pub(crate) fn published_at(&self, published: &DateTime) -> bool {
        if let Some(since) = &self.since {
            if *published < since.start() {
                return false;
            }
        }

        if let Some(until) = &self.until {
            if *published > until.end() {
                return false;
            }
        }

        true
    }

    pub(crate) fn limit_results<T>(&self, rows: &mut Vec<T>) {
        if let Some(limit) = self.limit {
            rows.truncate(limit);
        }
    }
}

/// How a column renders: `Pin` holds it at its content width so wrapping falls on
/// the other columns, `Num` right-aligns it.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ColumnSpec {
    Plain,
    Pin,
    Num,
}

pub(crate) fn detail_table() -> Table {
    let mut table = Table::new();
    table
        .load_style(UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Dynamic);
    table
}

pub(crate) fn table<'a, T>(columns: T) -> Table
where
    T: IntoIterator<Item = (&'a str, ColumnSpec)>,
{
    let (names, specs): (Vec<&str>, Vec<ColumnSpec>) = columns.into_iter().unzip();

    let mut table = detail_table();
    table.set_header(names);

    for (column, spec) in table.column_iter_mut().zip(specs) {
        match spec {
            ColumnSpec::Plain => {}
            ColumnSpec::Pin => {
                column.set_constraint(ColumnConstraint::ContentWidth);
            }
            ColumnSpec::Num => {
                column.set_cell_alignment(CellAlignment::Right);
            }
        }
    }

    table
}

/// Spotify ids are `spotify:<kind>:<id>`; only the trailing id carries information.
pub(crate) fn short_id(id: &str) -> &str {
    match id.rsplit_once(':') {
        Some((_, id)) => id,
        None => id,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Timespec {
    Range { start: DateTime, end: DateTime },
    Instant(DateTime),
}

impl Timespec {
    fn start(&self) -> DateTime {
        match self {
            Self::Range { start, .. } => *start,
            Self::Instant(at) => *at,
        }
    }

    fn end(&self) -> DateTime {
        match self {
            Self::Range { end, .. } => *end,
            Self::Instant(at) => *at,
        }
    }

    fn range(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('-').collect();

        let start = match parts[..] {
            [y] => NaiveDate::from_ymd_opt(y.parse().ok()?, 1, 1),
            [y, m] => NaiveDate::from_ymd_opt(y.parse().ok()?, m.parse().ok()?, 1),
            [y, m, d] => NaiveDate::from_ymd_opt(y.parse().ok()?, m.parse().ok()?, d.parse().ok()?),
            _ => None,
        }?;

        let next = match parts.len() {
            1 => start.checked_add_months(Months::new(12))?,
            2 => start.checked_add_months(Months::new(1))?,
            _ => start.succ_opt()?,
        };

        Some(Self::Range {
            start: start.and_hms_opt(0, 0, 0)?.and_utc(),
            end: next
                .and_hms_opt(0, 0, 0)?
                .and_utc()
                .checked_sub_signed(TimeDelta::nanoseconds(1))?,
        })
    }

    fn instant(interval: Interval) -> Option<Self> {
        let now = Utc::now();

        let at = match interval {
            Interval::Seconds(secs) => {
                now.checked_sub_signed(TimeDelta::seconds(i64::from(secs).abs()))?
            }
            Interval::Days(days) => {
                now.checked_sub_signed(TimeDelta::days(i64::from(days).abs()))?
            }
            Interval::Months(months) => {
                now.checked_sub_months(Months::new(months.unsigned_abs()))?
            }
        };

        Some(Self::Instant(at))
    }
}

impl FromStr for Timespec {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(range) = Self::range(s) {
            return Ok(range);
        }

        if let Ok(interval) = interim::parse_duration(s) {
            return Self::instant(interval)
                .ok_or_else(|| anyhow!("timespec {s:?} resolves out of range"));
        }

        let at = interim::parse_date_string(s, Local::now(), Dialect::Us)
            .map_err(|e| anyhow!("invalid timespec {s:?}: {e}"))?;

        Ok(Self::Instant(at.with_timezone(&Utc)))
    }
}
