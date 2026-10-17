//! The lyncast binary. Until step 5 it has one subcommand: `query` scans the desktop entries,
//! ranks them in lyncast-core with no visits and the system clock, and prints one JSON object
//! per row.

mod apps;
mod view;

use std::io::{self, Write};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};
use lyncast_core::rank::{Index, Timestamp, Visits};

#[derive(Parser)]
#[command(name = "lyncast", version, about = "A fast, small launcher for niri.")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the root list for a query, one row per line.
    // clap reads `-h` as help even with `allow_hyphen_values`, so only `--help` is help here.
    #[command(disable_help_flag = true)]
    Query {
        /// The query as typed. It may start with a hyphen, as in `lyncast query -5+3`. Write
        /// `lyncast query --json -- <text>` when the text is `--`, `--json`, or `--help`.
        #[arg(allow_hyphen_values = true)]
        text: String,
        /// Print each row as one compact JSON object. Required until a plain view exists.
        #[arg(long, required = true)]
        json: bool,
        /// Print help.
        #[arg(long, action = clap::ArgAction::Help)]
        help: Option<bool>,
    },
}

/// Exits 0 on success, 1 when stdout cannot be written, and 2 on a usage error.
fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => return clap_exit(&error),
    };
    match cli.command {
        Cmd::Query { text, .. } => query(&text),
    }
}

/// Prints clap's help, version, or usage error with clap's exit code. Help and version go to
/// stdout, so a failed write there exits 1, where `Error::exit` would ignore it and exit 0.
fn clap_exit(error: &clap::Error) -> ExitCode {
    let printed = error.print().and_then(|()| io::stdout().flush());
    if printed.is_err() && !error.use_stderr() {
        return ExitCode::FAILURE;
    }
    ExitCode::from(u8::try_from(error.exit_code()).unwrap_or(1))
}

fn query(text: &str) -> ExitCode {
    let index = Index::new(apps::scan(&apps::Vars::read()));
    let rows = index.rows(text, &Visits::default(), now());
    let mut out = io::stdout().lock();
    match view::write_rows(&mut out, &rows).and_then(|()| out.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // eprintln! panics when stderr fails too, which would exit 101.
            let _ = writeln!(io::stderr(), "lyncast: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The one clock read per query. A clock before 1970 reads as the epoch.
fn now() -> Timestamp {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp(i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}
