//! The root list for apps and built-in commands, ranked by TinyCast's matching rules and
//! frecency. [`Index::rows`] gives Results for a typed query, and Suggestions then Commands
//! for the empty query. The rules and their fixtures are in docs/design/ranking.md.

mod frecency;
mod key;
mod score;

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::iter;

pub use frecency::{Timestamp, Visit, Visits};
pub use score::{Fit, Scored, score};

use crate::desktop::App;
use crate::{Command, Key, Row, Section, Subject, fold};
use frecency::Learned;
use key::{Alpha, Hits, KindRank, Name, RankKey, Tiebreak};
use score::{Query, field_end};

/// The most rows Suggestions holds.
const SUGGESTIONS: usize = 5;

/// The longest query text that [`Index::rows`] folds, as many bytes as a field keeps. Longer
/// text could match only when the fold or the needle makes it shorter, as with full-width forms,
/// which fold from 3 bytes to 1, or a run of combining marks.
const MAX_QUERY_BYTES: usize = 1024;

/// Every searchable entry, folded once. A rescan builds a new one.
#[derive(Clone, Debug)]
pub struct Index {
    /// In [`Name`] order, so an entry's position is its [`Alpha`].
    entries: Vec<Entry>,
    /// Every field of every entry, contiguous per entry and in entry order.
    haystacks: Vec<Haystack>,
    by_key: HashMap<Key, Alpha>,
    /// The most characters in one haystack. A longer needle cannot align.
    longest: usize,
}

#[derive(Clone, Debug)]
enum Entry {
    App(Box<App>),
    Command(Command),
}

/// One field of one entry.
#[derive(Clone, Debug)]
struct Haystack {
    /// The field's fold with case kept, cut to its first 1,024 bytes. frizbee scores this.
    kept: Box<str>,
    /// The whole field's fold, lowercased and trimmed, as the query is. Exactness and prefixes
    /// compare this, so a cut field is exact only when all of it equals the query.
    lower: Box<str>,
    entry: Alpha,
    field: Field,
}

impl AsRef<str> for Haystack {
    fn as_ref(&self) -> &str {
        &self.kept
    }
}

/// Which of an entry's fields a haystack is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Field {
    /// An app's `Name` in the session locale, or a command's title.
    Title,
    /// An app's unlocalized `Name`, kept only when it folds differently from the title.
    AltTitle,
    /// An app's `GenericName` in the session locale.
    Subtitle,
    /// One keyword, or an app's title and subtitle joined. It makes the entry show and never
    /// ranks it.
    Keyword,
}

impl Index {
    /// Indexes these apps and every built-in command. A repeated app ID keeps its first app.
    pub fn new(apps: Vec<App>) -> Index {
        let mut ids = HashSet::new();
        let mut entries: Vec<Entry> = apps
            .into_iter()
            .filter(|app| ids.insert(app.id().clone()))
            .map(|app| Entry::App(Box::new(app)))
            .chain(Command::ALL.map(Entry::Command))
            .collect();
        entries.sort_by_cached_key(|entry| Name {
            folded: fold(entry.title()).lower().to_owned(),
            raw: entry.title().to_owned(),
            key: entry.key().to_string(),
        });
        let mut haystacks = Vec::new();
        let mut by_key = HashMap::new();
        for (at, entry) in entries.iter().enumerate() {
            let alpha = Alpha(at as u32);
            let first = haystacks.len();
            for (field, text) in entry.fields() {
                let (mut kept, lower) = fold(&text).into_parts();
                kept.truncate(field_end(&kept));
                let lower = lower.trim();
                let repeats = |earlier: &Haystack| {
                    *earlier.lower == *lower
                        && matches!(
                            (earlier.field, field),
                            (Field::Title, Field::AltTitle) | (Field::Keyword, Field::Keyword)
                        )
                };
                if lower.is_empty() || haystacks[first..].iter().any(repeats) {
                    continue;
                }
                haystacks.push(Haystack {
                    kept: kept.into(),
                    lower: lower.into(),
                    entry: alpha,
                    field,
                });
            }
            by_key.insert(entry.key(), alpha);
        }
        let longest = haystacks
            .iter()
            .map(|haystack| haystack.kept.chars().count())
            .max()
            .unwrap_or(0);
        Index {
            entries,
            haystacks,
            by_key,
            longest,
        }
    }

    /// The root list for `query` at `now`. Text over 1,024 bytes gives no rows, before the
    /// fold. Text that is empty or only whitespace once folded gives Suggestions then Commands,
    /// so a lone soft hyphen does too. Any other text gives the Results that show, best first.
    pub fn rows(&self, query: &str, visits: &Visits, now: Timestamp) -> Vec<Row> {
        if query.len() > MAX_QUERY_BYTES {
            return Vec::new();
        }
        let learned = self.learned(visits, now);
        match Query::new(query) {
            Some(query) => self.typed(&query, &learned),
            None => self.empty(&learned),
        }
    }

    /// One frizbee pass over the haystacks, then one sort of the shown entries' keys. A needle
    /// longer than every haystack returns before frizbee builds its matcher, whose setup grows
    /// with the needle.
    fn typed(&self, query: &Query, learned: &[Learned<'_>]) -> Vec<Row> {
        if query.len() > self.longest {
            return Vec::new();
        }
        let Some(mut matcher) = query.matcher() else {
            return Vec::new();
        };
        let matches = matcher.match_list(&self.haystacks);
        let entry_of = |index: u32| self.haystacks[index as usize].entry;
        let mut keys = Vec::new();
        for run in matches.chunk_by(|a, b| entry_of(a.index) == entry_of(b.index)) {
            let mut hits = Hits::default();
            for found in run {
                let haystack = &self.haystacks[found.index as usize];
                hits.add(haystack.field, &haystack.lower, found.score, query);
            }
            if hits.shows {
                let alpha = entry_of(run[0].index);
                let entry = learned[alpha.0 as usize];
                keys.push(RankKey::new(
                    &hits,
                    entry,
                    query,
                    self.tiebreak(alpha, entry),
                ));
            }
        }
        keys.sort_unstable();
        keys.iter()
            .map(|key| self.row(key.alpha(), Section::Results))
            .collect()
    }

    /// Suggestions holds the entries whose frecency is above 1, by the tiebreak, then
    /// Clipboard History, then apps in name order, up to 5. Commands holds every other command
    /// in name order.
    fn empty(&self, learned: &[Learned<'_>]) -> Vec<Row> {
        let alphas = (0..self.entries.len() as u32).map(Alpha);
        let mut warm: Vec<Tiebreak> = alphas
            .clone()
            .filter(|alpha| learned[alpha.0 as usize].frecency.is_warm())
            .map(|alpha| self.tiebreak(alpha, learned[alpha.0 as usize]))
            .collect();
        warm.sort_unstable();
        let mut picked: Vec<Alpha> = warm.iter().take(SUGGESTIONS).map(Tiebreak::alpha).collect();
        let clipboard = self.by_key[&Key::Command(Command::ClipboardHistory)];
        let apps = alphas
            .clone()
            .filter(|&alpha| matches!(self.entry(alpha), Entry::App(_)));
        for alpha in iter::once(clipboard).chain(apps) {
            if picked.len() == SUGGESTIONS {
                break;
            }
            if !picked.contains(&alpha) {
                picked.push(alpha);
            }
        }
        let commands = alphas.filter(|&alpha| {
            matches!(self.entry(alpha), Entry::Command(_)) && !picked.contains(&alpha)
        });
        picked
            .iter()
            .map(|&alpha| self.row(alpha, Section::Suggestions))
            .chain(commands.map(|alpha| self.row(alpha, Section::Commands)))
            .collect()
    }

    /// Every entry's learned ranking at `now`, by position, with one map lookup per visit.
    fn learned<'v>(&self, visits: &'v Visits, now: Timestamp) -> Vec<Learned<'v>> {
        let mut learned = vec![Learned::default(); self.entries.len()];
        for (key, visit) in visits.iter() {
            if let Some(alpha) = self.by_key.get(key) {
                learned[alpha.0 as usize] = visit.learned(now);
            }
        }
        learned
    }

    fn tiebreak(&self, alpha: Alpha, learned: Learned<'_>) -> Tiebreak {
        Tiebreak::new(learned.frecency, self.entry(alpha).kind(), alpha)
    }

    fn row(&self, alpha: Alpha, section: Section) -> Row {
        let entry = self.entry(alpha);
        Row {
            subject: entry.subject(),
            section,
            title: entry.title().to_owned(),
            card: None,
        }
    }

    fn entry(&self, alpha: Alpha) -> &Entry {
        &self.entries[alpha.0 as usize]
    }
}

impl Entry {
    fn title(&self) -> &str {
        match self {
            Entry::App(app) => app.name(),
            Entry::Command(command) => command.title(),
        }
    }

    fn key(&self) -> Key {
        match self {
            Entry::App(app) => Key::App(app.id().clone()),
            Entry::Command(command) => Key::Command(*command),
        }
    }

    fn subject(&self) -> Subject {
        match self {
            Entry::App(app) => Subject::App {
                id: app.id().clone(),
                actions: app
                    .actions()
                    .iter()
                    .map(|action| action.action.clone())
                    .collect(),
            },
            Entry::Command(command) => Subject::Command(*command),
        }
    }

    fn kind(&self) -> KindRank {
        match self {
            Entry::App(_) => KindRank::App,
            Entry::Command(_) => KindRank::Command,
        }
    }

    /// The searchable texts, title first, before the fold. An app with a subtitle also has the
    /// title and subtitle joined both ways as keywords, as TinyCast's keywords do.
    fn fields(&self) -> Vec<(Field, Cow<'_, str>)> {
        match self {
            Entry::App(app) => {
                let title = app.name();
                let mut fields = vec![
                    (Field::Title, Cow::Borrowed(title)),
                    (Field::AltTitle, Cow::Borrowed(app.unlocalized_name())),
                ];
                if let Some(generic) = app.generic_name() {
                    fields.extend([
                        (Field::Subtitle, Cow::Borrowed(generic)),
                        (Field::Keyword, Cow::Owned(format!("{title} {generic}"))),
                        (Field::Keyword, Cow::Owned(format!("{generic} {title}"))),
                    ]);
                }
                let keywords = app.keywords().iter();
                fields.extend(keywords.map(|keyword| (Field::Keyword, Cow::Borrowed(&**keyword))));
                fields
            }
            Entry::Command(command) => iter::once((Field::Title, command.title()))
                .chain(
                    command
                        .keywords()
                        .iter()
                        .map(|&keyword| (Field::Keyword, keyword)),
                )
                .map(|(field, text)| (field, Cow::Borrowed(text)))
                .collect(),
        }
    }
}
