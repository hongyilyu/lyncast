use std::path::Path;

use lyncast_core::clip::TextKind;
use lyncast_core::rank::Timestamp;
use lyncast_core::{ClipId, Subject, fold};

use crate::clip::{Content, Row};

const TITLE_CHARS: usize = 200;

/// What the type filter keeps. Ctrl+T moves it to the next, after Emails Only back to All
/// Types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kinds {
    All,
    /// Text, with the links and emails among it.
    Text,
    Images,
    Files,
    Links,
    Emails,
}

impl Kinds {
    pub fn next(self) -> Kinds {
        match self {
            Kinds::All => Kinds::Text,
            Kinds::Text => Kinds::Images,
            Kinds::Images => Kinds::Files,
            Kinds::Files => Kinds::Links,
            Kinds::Links => Kinds::Emails,
            Kinds::Emails => Kinds::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kinds::All => "All Types",
            Kinds::Text => "Text Only",
            Kinds::Images => "Images Only",
            Kinds::Files => "Files Only",
            Kinds::Links => "Links Only",
            Kinds::Emails => "Emails Only",
        }
    }

    fn keeps(self, content: Content) -> bool {
        match self {
            Kinds::All => true,
            Kinds::Text => matches!(content, Content::Text(_)),
            Kinds::Images => content == Content::Image,
            Kinds::Files => content == Content::Files,
            Kinds::Links => content == Content::Text(TextKind::Link),
            Kinds::Emails => content == Content::Text(TextKind::Email),
        }
    }
}

/// A date and a time of day on a clock's wall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Civil {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

impl Civil {
    /// The wall time of `at` in a zone `offset` seconds east of UTC.
    pub fn at(at: Timestamp, offset: i64) -> Civil {
        let seconds = at.0.div_euclid(1000) + offset;
        let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
        let of_day = seconds.rem_euclid(86_400);
        Civil {
            year,
            month,
            day,
            hour: (of_day / 3600) as u32,
            minute: (of_day % 3600 / 60) as u32,
        }
    }

    /// The days from 1970-01-01 to this date.
    fn days(self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }

    /// The time of this wall time in a zone `offset` seconds east of UTC.
    #[cfg(test)]
    pub fn timestamp(self, offset: i64) -> Timestamp {
        let minutes = (self.days() * 24 + i64::from(self.hour)) * 60 + i64::from(self.minute);
        Timestamp((minutes * 60 - offset) * 1000)
    }
}

/// The group a clip shows under, and the day its Copied line names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Day {
    Pinned,
    Today,
    Yesterday,
    /// Any other day, with its year when that is not this year.
    Date {
        month: u32,
        day: u32,
        year: Option<i64>,
    },
}

impl Day {
    fn of(date: Civil, today: Civil) -> Day {
        match today.days() - date.days() {
            0 => Day::Today,
            1 => Day::Yesterday,
            _ => Day::Date {
                month: date.month,
                day: date.day,
                year: (date.year != today.year).then_some(date.year),
            },
        }
    }

    pub fn label(self) -> String {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        match self {
            Day::Pinned => "Pinned".to_owned(),
            Day::Today => "Today".to_owned(),
            Day::Yesterday => "Yesterday".to_owned(),
            Day::Date { month, day, year } => {
                let month = MONTHS[month as usize - 1];
                match year {
                    Some(year) => format!("{month} {day}, {year}"),
                    None => format!("{month} {day}"),
                }
            }
        }
    }
}

/// What the clipboard screen shows of one clip: its line in the list and its information. It
/// holds no bytes of the clip.
pub struct ClipLine {
    pub id: ClipId,
    pub pinned: bool,
    pub content: Content,
    pub title: String,
    /// The group it shows under: Pinned, or the day of its last use.
    pub day: Day,
}

impl ClipLine {
    /// The subject the chord table reads, so the menu and Enter act on the row.
    pub fn subject(&self) -> Subject {
        Subject::Clip {
            id: self.id,
            pinned: self.pinned,
        }
    }

    /// The letter of the square that the list draws beside the clip.
    pub fn letter(&self) -> &'static str {
        match self.content {
            Content::Text(TextKind::Text) => "T",
            Content::Text(TextKind::Link) => "L",
            Content::Text(TextKind::Email) => "@",
            Content::Image => "I",
            Content::Files => "F",
        }
    }
}

/// The clipboard screen's rows from the history in screen order: pins first, then the rest by
/// last use, newest first. A row stays when the type filter keeps its kind and its preview holds
/// every whitespace-separated word of the filter, both folded as ranking folds a query. An image
/// has no preview, so only an empty filter keeps it. `date` gives a time's wall time, and `today`
/// is the clock's wall time now.
pub fn clip_lines(
    rows: &[&Row],
    filter: &str,
    kinds: Kinds,
    date: impl Fn(Timestamp) -> Civil,
    today: Civil,
) -> Vec<ClipLine> {
    let filter = fold(filter);
    let words: Vec<&str> = filter.lower().split_whitespace().collect();
    let found = |row: &Row| {
        words.is_empty()
            || row.preview.as_deref().is_some_and(|preview| {
                let preview = fold(preview);
                words.iter().all(|word| preview.lower().contains(word))
            })
    };
    rows.iter()
        .filter(|row| kinds.keeps(row.content) && found(row))
        .map(|row| {
            let used = Day::of(date(row.used()), today);
            ClipLine {
                id: row.id,
                pinned: row.pinned,
                content: row.content,
                title: title(row),
                day: if row.pinned { Day::Pinned } else { used },
            }
        })
        .collect()
}

fn title(row: &Row) -> String {
    let preview = row.preview.as_deref().unwrap_or_default();
    match row.content {
        Content::Text(_) => {
            let line = preview.lines().find(|line| !line.trim().is_empty());
            line.unwrap_or_default()
                .trim()
                .chars()
                .take(TITLE_CHARS)
                .collect()
        }
        Content::Files => {
            let mut paths = preview.lines();
            let first = paths.next().unwrap_or_default();
            let name = Path::new(first)
                .file_name()
                .map_or(first.into(), |name| name.to_string_lossy());
            match paths.count() {
                0 => name.into_owned(),
                more => format!("{name} +{more}"),
            }
        }
        Content::Image => "Image".to_owned(),
    }
}

/// The year, month, and day of the day `days` after 1970-01-01, in the proleptic Gregorian
/// calendar (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let of_era = z.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let from_march = (5 * of_year + 2) / 153;
    let day = (of_year - (153 * from_march + 2) / 5 + 1) as u32;
    let month = if from_march < 10 {
        from_march + 3
    } else {
        from_march - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let from_march = i64::from((month + 9) % 12);
    let of_year = (153 * from_march + 2) / 5 + i64::from(day) - 1;
    let of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + of_year;
    era * 146_097 + of_era - 719_468
}
