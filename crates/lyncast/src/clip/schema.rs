use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};

use super::blob::Hash;
use super::{Content, Seq};

/// The schema that the writer creates in one transaction before its first change, which
/// `schema.sql` in docs/design/clipboard-and-paste.md holds too. A file that holds anything loads
/// only when it holds what this makes.
pub const SCHEMA: &str = "PRAGMA application_id = 1819897443;
CREATE TABLE IF NOT EXISTS clips (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL CHECK (kind IN ('text', 'link', 'email', 'image', 'files')),
    mime TEXT NOT NULL,
    bytes INTEGER NOT NULL CHECK (bytes >= 0),
    body TEXT,
    hash TEXT NOT NULL,
    preview TEXT,
    created INTEGER NOT NULL,
    used INTEGER NOT NULL,
    pinned INTEGER NOT NULL CHECK (pinned IN (0, 1)),
    seq INTEGER NOT NULL CHECK (seq >= 0),
    CHECK ((kind = 'image') = (body IS NULL)),
    CHECK ((kind = 'image') = (preview IS NULL))
) STRICT;";

impl ToSql for Content {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.spelling().into())
    }
}

impl FromSql for Content {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Content> {
        Content::read(value.as_str()?).ok_or(FromSqlError::InvalidType)
    }
}

impl ToSql for Hash {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.to_string().into())
    }
}

impl FromSql for Hash {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Hash> {
        Hash::from_hex(value.as_str()?).ok_or(FromSqlError::InvalidType)
    }
}

impl ToSql for Seq {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.0.into())
    }
}

impl FromSql for Seq {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Seq> {
        let seq = value.as_i64()?;
        if seq < 0 {
            return Err(FromSqlError::OutOfRange(seq));
        }
        Ok(Seq(seq))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc;

    #[test]
    fn the_doc_holds_the_schema() {
        let blocks = doc::CLIPBOARD.blocks("file");
        let schema: Vec<&doc::Block> = blocks
            .iter()
            .filter(|block| block.args == ["schema.sql"])
            .collect();
        let [schema] = schema.as_slice() else {
            panic!("clipboard-and-paste.md holds one schema.sql block");
        };
        assert_eq!(schema.lines.join("\n"), SCHEMA);
    }

    #[test]
    fn the_schema_takes_the_kinds_that_the_daemon_spells() {
        let kinds = Content::ALL.map(|content| format!("'{}'", content.spelling()));
        let check = format!("CHECK (kind IN ({}))", kinds.join(", "));
        assert!(SCHEMA.contains(&check), "SCHEMA lacks {check}");
    }
}
