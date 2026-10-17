//! The lyncast binary. `lyncast daemon --headless` runs the daemon, which scans the desktop
//! entries, owns the socket, and builds the root list in lyncast-core. The other subcommands
//! are clients that send it one request each and print the reply (docs/design/daemon.md).

mod apps;
mod client;
mod daemon;
mod files;
#[cfg(test)]
#[path = "../tests/common/pipe.rs"]
mod pipe;
mod vars;
mod view;
mod warning;
mod wire;

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use lyncast_core::{Chord, Key, Verb};

use view::Spelled;
use wire::{Reply, Request, Via};

#[derive(Parser)]
#[command(name = "lyncast", version, about = "A fast, small launcher for niri.")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the daemon, which scans the apps and answers on $XDG_RUNTIME_DIR/lyncast.sock.
    Daemon {
        /// Run without the palette. Required until the palette exists.
        #[arg(long, required = true)]
        headless: bool,
        /// Append each program the daemon would start to FILE, as a JSON array, and start none.
        #[arg(long, value_name = "FILE")]
        record: Option<PathBuf>,
    },
    /// Open the summon if it is closed, or close it, and print the new state.
    Toggle,
    /// Print the summon state, open or closed.
    Status,
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
    /// Run one action of a row, and print what happened.
    Activate {
        /// The row's key, such as `app:firefox.desktop`, `cmd:reboot`, or `calc:-5+3`.
        #[arg(value_parser = Key::read)]
        key: Key,
        /// The keys pressed: `enter` or `ctrl-enter`. Without `--verb`, Enter.
        #[arg(long, value_parser = Chord::read, conflicts_with = "verb")]
        chord: Option<Chord>,
        /// An item of the row's Actions menu, spelled as `query --json` spells its verb.
        #[arg(long, value_parser = Verb::read)]
        verb: Option<Verb>,
        /// Confirm a command that asks first, such as Reboot.
        #[arg(long)]
        yes: bool,
        /// The search text the row was found with, which frecency keeps as a search term.
        #[arg(long, default_value = "", allow_hyphen_values = true)]
        query: String,
    },
}

/// Exits 0 on success, 1 when the request fails or stdout cannot be written, and 2 on a usage
/// error.
fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => return clap_exit(&error),
    };
    let done = match cli.command {
        Cmd::Daemon { record, .. } => return daemon::run(record),
        Cmd::Toggle => summon(&Request::Toggle),
        Cmd::Status => summon(&Request::Status),
        Cmd::Query { text, .. } => query(text),
        Cmd::Activate {
            key,
            chord,
            verb,
            yes,
            query,
        } => activate(Request::Activate {
            key,
            via: verb.map_or(Via::Chord(chord.unwrap_or(Chord::Enter)), Via::Menu),
            confirmed: yes,
            query,
        }),
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(reason) => {
            // eprintln! panics when stderr fails too, which would exit 101.
            let _ = writeln!(io::stderr(), "lyncast: {reason}");
            ExitCode::FAILURE
        }
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

/// `toggle` and `status` print the summon state.
fn summon(request: &Request) -> Result<(), String> {
    match client::send(request)? {
        Reply::Summon { state } => print(|out| writeln!(out, "{state}")),
        reply => Err(client::refusal(reply)),
    }
}

fn query(text: String) -> Result<(), String> {
    match client::send(&Request::Query { text })? {
        Reply::Rows { rows } => print(|out| view::write_rows(out, &rows)),
        reply => Err(client::refusal(reply)),
    }
}

/// Prints the success line. A command that asks first prints its question as the failure.
fn activate(request: Request) -> Result<(), String> {
    match client::send(&request)? {
        Reply::Done { message } => print(|out| writeln!(out, "{message}")),
        Reply::Confirm { title } => Err(format!("{title}? Run again with --yes to confirm.")),
        reply => Err(client::refusal(reply)),
    }
}

/// Writes to stdout and flushes it, so a failed write is the command's failure.
fn print(write: impl FnOnce(&mut io::StdoutLock<'_>) -> io::Result<()>) -> Result<(), String> {
    let mut out = io::stdout().lock();
    write(&mut out)
        .and_then(|()| out.flush())
        .map_err(|error| error.to_string())
}
