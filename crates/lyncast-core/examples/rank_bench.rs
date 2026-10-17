//! Ranking's cost per keystroke. Run it with
//! `cargo run --release -p lyncast-core --example rank_bench`, or loop one group for a profiler
//! with `-- profile <apps> <group>`, such as `-- profile 1000 1`.
//!
//! The index holds 300 or 1,000 apps built through `desktop::parse`. The names are the 216
//! real app names in bench/score-map, then two-word names drawn from 40 brand words and 40
//! nouns. 3 in 4 apps get one of 50 generic names, and each app gets 0 to 6 keywords from a
//! 150-word vocabulary, skewed so a few words are common. Every tenth app has 1 to 4 visits
//! in the last 30 days, half with a search term.
//!
//! The queries are what a person types: every prefix, up to 8 characters, of one app name in
//! 37, plus five common names and two misses, grouped by length. A long group adds a 64- and a
//! 128-character sentence and a 1,024-byte paste. Each of 7 runs repeats one loop for about
//! 50 ms, and each iteration times three passes back to back: `Index::rows` over the group,
//! frizbee alone on the bench's own copy of the haystacks, and cloning and dropping the returned
//! rows, which makes the same allocations as building them. The remainder is derived, not
//! timed: each iteration's `Index::rows` time minus its other two, averaged over the run. Each
//! call to `Index::rows` is also timed on its own inside the loop, so the report names the
//! group's worst query.
//!
//! A last table times frizbee on one field as long as each long needle, the cost a desktop
//! with such a field pays, since the index skips frizbee only for a needle longer than every
//! field.

use std::hint::black_box;
use std::time::{Duration, Instant};

use frizbee::{CaseMatching, Config, Matcher, Scoring, SortStrategy};
use lyncast_core::desktop::{self, App, Session, Verdict};
use lyncast_core::rank::{Index, Timestamp, Visits};
use lyncast_core::{AppId, Command, Key, Row, fold};

const REAL_NAMES: &str = include_str!("../../../bench/score-map/src/names.txt");

const BRANDS: &str = "\
    Aurora Beacon Cinder Delta Ember Falcon Glacier Harbor Iris Juniper Kestrel Lumen Meridian \
    Nimbus Onyx Pulsar Quill Raven Sable Tundra Umber Vertex Willow Xenon Yarrow Zephyr Atlas \
    Borealis Cobalt Dune Echo Fjord Garnet Helix Indigo Jasper Krypton Lotus Mosaic Nova";

const NOUNS: &str = "\
    Notes Mail Studio Player Viewer Editor Reader Writer Draw Paint Sync Backup Monitor Terminal \
    Shell Browser Chat Meet Maps Weather Clock Calendar Contacts Photos Music Radio Podcasts \
    Books Scan Print Tasks Board Sheets Slides Docs Vault Keys Tunnel Desk Launcher";

/// Separated by `;`, since they hold spaces.
const GENERIC: &str = "\
    Web Browser;Text Editor;Terminal Emulator;File Manager;Image Viewer;Media Player;\
    Music Player;Video Editor;Photo Editor;Office Suite;Word Processor;Spreadsheet;Presentation;\
    Email Client;Instant Messenger;Video Conferencing;Password Manager;Archive Manager;\
    PDF Viewer;E-book Reader;Calculator;Calendar;Note Taking;Screenshot Tool;Screen Recorder;\
    System Monitor;Disk Utility;Partition Editor;Network Manager;Bluetooth Manager;Audio Mixer;\
    Torrent Client;FTP Client;File Synchronizer;Backup Tool;Integrated Development Environment;\
    Database Client;API Client;Version Control;Virtual Machine Manager;Remote Desktop;\
    Game Launcher;Font Viewer;Character Map;Color Picker;Document Scanner;Podcast Player;\
    Feed Reader;Vector Graphics Editor;Digital Audio Workstation";

const KEYWORDS: &str = "\
    internet web browser mail email chat messaging video audio music sound player media movie \
    photo image picture graphics drawing paint vector raster 3d modeling animation editor text \
    code programming development ide debugger compiler git vcs terminal shell console command \
    prompt files folder explorer manager archive zip compress backup sync cloud storage disk \
    partition system monitor process task cpu memory network wifi bluetooth vpn remote desktop \
    vnc rdp ssh ftp torrent download upload office document spreadsheet presentation word pdf \
    ebook reader notes todo calendar schedule contacts address clock timer alarm weather \
    forecast map navigation gps calculator math science education game gaming emulator launcher \
    steam store software update package install settings preferences configuration theme font \
    color screenshot capture record webcam camera scanner printer password security encryption \
    keys privacy virtual machine container docker database sql api http rss news podcast radio \
    stream www kde gnome gtk qt utility accessories viewer voice suite wallpaper";

const DAY: i64 = 86_400_000;
const NOW: Timestamp = Timestamp(1_791_072_000_000);

/// Timed runs per measurement. The report gives their median and range.
const RUNS: usize = 7;

/// A fixed-seed LCG, so every run indexes the same apps.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % n as u64) as usize
    }

    /// Skewed toward 0, so a few keywords are common and most are rare.
    fn skewed(&mut self, n: usize) -> usize {
        let a = self.below(n);
        let b = self.below(n);
        a * b / n
    }
}

/// One word of a space-separated list.
fn pick<'a>(words: &'a str, rng: &mut Lcg) -> &'a str {
    let words: Vec<&str> = words.split_whitespace().collect();
    words[rng.below(words.len())]
}

fn names(n: usize, rng: &mut Lcg) -> Vec<String> {
    let commands: Vec<&str> = Command::ALL.map(Command::title).to_vec();
    let mut names: Vec<String> = REAL_NAMES
        .lines()
        .filter(|name| !name.is_empty() && !commands.contains(name))
        .map(str::to_owned)
        .take(n)
        .collect();
    while names.len() < n {
        let mut name = format!("{} {}", pick(BRANDS, rng), pick(NOUNS, rng));
        if rng.below(5) == 0 {
            name.push_str([" Pro", " Lite", " 2", " Nightly"][rng.below(4)]);
        }
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

fn synthetic(n: usize) -> (Vec<App>, Visits) {
    let generic: Vec<&str> = GENERIC.split(';').collect();
    let vocabulary: Vec<&str> = KEYWORDS.split_whitespace().collect();
    let mut rng = Lcg(42);
    let session = Session::new(&["C"], "niri");
    let mut apps = Vec::new();
    let mut visits = Visits::default();
    for (at, name) in names(n, &mut rng).into_iter().enumerate() {
        let mut text = format!("[Desktop Entry]\nType=Application\nName={name}\nExec=app{at} %U\n");
        if rng.below(4) != 0 {
            let generic = generic[rng.below(generic.len())];
            text.push_str(&format!("GenericName={generic}\n"));
        }
        let words: Vec<&str> = (0..rng.below(7))
            .map(|_| vocabulary[rng.skewed(vocabulary.len())])
            .collect();
        if !words.is_empty() {
            text.push_str(&format!("Keywords={};\n", words.join(";")));
        }
        let id = format!("app{at}.desktop");
        let path = format!("/usr/share/applications/{id}");
        let Ok(Verdict::Shown(app)) = desktop::parse(AppId(id.clone()), &path, &text, &session)
        else {
            panic!("{name} does not parse");
        };
        if at % 10 == 0 {
            let lower = name.to_lowercase();
            for _ in 0..1 + rng.below(4) {
                let term: String = lower.chars().take(2 + rng.below(4)).collect();
                let query = if rng.below(2) == 0 { term.as_str() } else { "" };
                let day = NOW.0 - rng.below(30) as i64 * DAY;
                visits.record(Key::App(AppId(id.clone())), query, Timestamp(day));
            }
        }
        apps.push(*app);
    }
    (apps, visits)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    AltTitle,
    Subtitle,
    Keyword,
}

/// The bench's own copy of the index's haystacks: each entry's folded title, its alternate
/// title when it folds differently, its subtitle, the title and subtitle joined both ways, and
/// its distinct keywords.
fn haystacks(apps: &[App]) -> Vec<String> {
    let apps = apps.iter().map(|app| {
        let title = app.name().to_owned();
        let mut fields = vec![
            (Field::Title, title.clone()),
            (Field::AltTitle, app.unlocalized_name().to_owned()),
        ];
        if let Some(generic) = app.generic_name() {
            fields.extend([
                (Field::Subtitle, generic.to_owned()),
                (Field::Keyword, format!("{title} {generic}")),
                (Field::Keyword, format!("{generic} {title}")),
            ]);
        }
        fields.extend(app.keywords().iter().map(|k| (Field::Keyword, k.clone())));
        fields
    });
    let commands = Command::ALL.map(|command| {
        let mut fields = vec![(Field::Title, command.title().to_owned())];
        fields.extend(
            command
                .keywords()
                .iter()
                .map(|&k| (Field::Keyword, k.to_owned())),
        );
        fields
    });
    let mut all = Vec::new();
    for fields in apps.chain(commands) {
        let mut seen: Vec<(Field, String)> = Vec::new();
        for (field, text) in fields {
            let folded = fold(&text);
            let repeats = seen.iter().any(|(earlier, lower)| {
                let same_kind = match field {
                    Field::AltTitle => *earlier == Field::Title,
                    Field::Keyword => *earlier == Field::Keyword,
                    Field::Title | Field::Subtitle => false,
                };
                same_kind && lower == folded.lower()
            });
            if folded.lower().is_empty() || repeats {
                continue;
            }
            seen.push((field, folded.lower().to_owned()));
            all.push(folded.kept().to_owned());
        }
    }
    all
}

/// What `Index::rows` hands frizbee for a typed query.
fn needle(query: &str) -> String {
    const SEPARATORS: [char; 10] = [' ', '\t', '\n', '-', '.', '/', '(', ')', '[', ']'];
    let folded = fold(query);
    folded
        .lower()
        .trim()
        .chars()
        .filter(|c| !SEPARATORS.contains(c))
        .collect()
}

fn config() -> Config {
    Config::default()
        .max_typos(Some(0))
        .casing(CaseMatching::Ignore)
        .sort(SortStrategy::IndexAsc)
        .scoring(Scoring {
            match_score: 2,
            prefix_bonus: 2,
            delimiter_bonus: 1,
            capitalization_bonus: 1,
            gap_open_penalty: 1,
            gap_extend_penalty: 0,
            mismatch_penalty: 4,
            matching_case_bonus: 0,
            exact_match_bonus: 0,
        })
}

/// The typed queries by group: 1, 2, 3, and 4 or more characters.
fn queries(apps: &[App]) -> [Vec<String>; 4] {
    let mut groups: [Vec<String>; 4] = Default::default();
    let common = [
        "firefox",
        "terminal",
        "calculator",
        "visual studio",
        "settings",
    ];
    let targets = apps
        .iter()
        .step_by(37)
        .map(|app| app.name().to_lowercase())
        .chain(common.map(str::to_owned));
    for target in targets {
        let chars: Vec<char> = target.chars().collect();
        for len in 1..=chars.len().min(8) {
            if chars[len - 1] != ' ' {
                groups[len.min(4) - 1].push(chars[..len].iter().collect());
            }
        }
    }
    groups[3].extend(["xyzq".to_owned(), "qqqq".to_owned()]);
    groups
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [mode, n, group] = args.as_slice()
        && mode == "profile"
    {
        return profile(n.parse().expect("an app count"), group);
    }
    println!(
        "{:<9} {:>5} {:>6}   {:<23} {:<34} {:<23} {:<23} {:<23}",
        "group",
        "apps",
        "shown",
        "rows µs",
        "worst query µs",
        "frizbee µs",
        "build rows µs",
        "remainder µs, derived"
    );
    let long = long_queries();
    for n in [300, 1000] {
        let (apps, visits) = synthetic(n);
        let hay = haystacks(&apps);
        let longest = hay.iter().map(|h| h.chars().count()).max().unwrap_or(0);
        let groups = queries(&apps);
        let index = Index::new(apps);
        let labels = ["1 char", "2 chars", "3 chars", "4+ chars"];
        for (label, group) in labels.into_iter().zip(&groups) {
            report(label, n, &index, &visits, &hay, longest, group);
        }
        report("long", n, &index, &visits, &hay, longest, &long);
        report("empty", n, &index, &visits, &hay, longest, &[String::new()]);
        println!(
            "{n} apps: {} haystacks, the longest {longest} characters",
            hay.len()
        );
    }
    println!("\nfrizbee on one field as long as the needle");
    for query in &long {
        let needle = needle(query);
        let field = [needle.clone()];
        let times = measure(1, || {
            black_box(Matcher::new(needle.as_str(), &config()).match_list(black_box(&field)));
        });
        println!(
            "{:>5} characters   {}",
            needle.chars().count(),
            spread(&times)
        );
    }
}

/// A 64- and a 128-character sentence, and a 1,024-byte paste.
fn long_queries() -> Vec<String> {
    let cycled = |len: usize| "firefox web browser ".chars().cycle().take(len).collect();
    vec![cycled(64), cycled(128), "firefox ".repeat(128)]
}

fn report(
    label: &str,
    n: usize,
    index: &Index,
    visits: &Visits,
    hay: &[String],
    longest: usize,
    group: &[String],
) {
    let results: Vec<Vec<Row>> = group.iter().map(|q| index.rows(q, visits, NOW)).collect();
    let shown: usize = results.iter().map(Vec::len).sum();
    // The index runs frizbee only for a needle that some field can hold.
    let needles: Vec<String> = group
        .iter()
        .map(|q| needle(q))
        .filter(|needle| !needle.is_empty() && needle.chars().count() <= longest)
        .collect();
    let mut matched = 0;
    for needle in &needles {
        matched += Matcher::new(needle.as_str(), &config())
            .match_list(hay)
            .len();
    }
    // A slip that showed nothing would time the miss path alone.
    assert!(
        (shown > 0 || label == "long") && (matched >= shown || label == "empty"),
        "{label}: {shown} shown"
    );
    // One iteration: the three passes back to back, each timed, and the remainder derived.
    let iteration = |calls: &mut [Duration]| -> [f64; 3] {
        let start = Instant::now();
        for (at, query) in group.iter().enumerate() {
            let call = Instant::now();
            black_box(index.rows(black_box(query), visits, NOW));
            calls[at] += call.elapsed();
        }
        let rows_done = Instant::now();
        for needle in &needles {
            black_box(Matcher::new(needle.as_str(), &config()).match_list(black_box(hay)));
        }
        let frizbee_done = Instant::now();
        for rows in &results {
            black_box(rows.clone());
        }
        let build_done = Instant::now();
        [
            rows_done - start,
            frizbee_done - rows_done,
            build_done - frizbee_done,
        ]
        .map(|pass| pass.as_secs_f64() * 1e6)
    };
    let reps = reps(|| {
        iteration(&mut vec![Duration::ZERO; group.len()]);
    });
    let calls_per_run = reps as f64 * group.len() as f64;
    let mut rows = Vec::new();
    let mut frizbee = Vec::new();
    let mut build = Vec::new();
    let mut remainder = Vec::new();
    // Each query's own time per call, one sample per run.
    let mut per_query = vec![Vec::new(); group.len()];
    for _ in 0..RUNS {
        let mut calls = vec![Duration::ZERO; group.len()];
        let mut sums = [0.0; 4];
        for _ in 0..reps {
            let [rows_pass, frizbee_pass, build_pass] = iteration(&mut calls);
            let remainder_pass = rows_pass - frizbee_pass - build_pass;
            for (sum, pass) in
                sums.iter_mut()
                    .zip([rows_pass, frizbee_pass, build_pass, remainder_pass])
            {
                *sum += pass;
            }
        }
        for (samples, call) in per_query.iter_mut().zip(calls) {
            samples.push(micros(call, reps as f64));
        }
        for (times, sum) in [&mut rows, &mut frizbee, &mut build, &mut remainder]
            .into_iter()
            .zip(sums)
        {
            times.push(sum / calls_per_run);
        }
    }
    let count = group.len() as f64;
    for times in [&mut rows, &mut frizbee, &mut build, &mut remainder] {
        times.sort_by(f64::total_cmp);
    }
    let (worst, query) = per_query
        .iter_mut()
        .zip(group)
        .map(|(samples, query)| {
            samples.sort_by(f64::total_cmp);
            (median(samples), query)
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .expect("a group has a query");
    let worst = match query.len() {
        0..=12 => format!("{worst:.2} ({query:?})"),
        bytes => format!("{worst:.2} ({bytes} bytes)"),
    };
    println!(
        "{label:<9} {n:>5} {:>6.1}   {:<23} {worst:<34} {:<23} {:<23} {:<23}",
        shown as f64 / count,
        spread(&rows),
        spread(&frizbee),
        spread(&build),
        spread(&remainder),
    );
}

/// Loops one group's `Index::rows` for 10 seconds, for a profiler to sample.
fn profile(n: usize, group: &str) {
    let (apps, visits) = synthetic(n);
    let groups = queries(&apps);
    let index = Index::new(apps);
    let group: Vec<String> = match group {
        "1" | "2" | "3" => groups[group.parse::<usize>().expect("a digit") - 1].clone(),
        "4+" => groups[3].clone(),
        "long" => long_queries(),
        _ => vec![String::new()],
    };
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        for query in &group {
            black_box(index.rows(black_box(query), &visits, NOW));
        }
    }
}

/// How many repetitions of `work` take about 50 ms.
fn reps(mut work: impl FnMut()) -> usize {
    let start = Instant::now();
    work();
    let once = start.elapsed().as_secs_f64();
    ((0.05 / once.max(1e-9)) as usize).max(1)
}

fn micros(elapsed: Duration, calls: f64) -> f64 {
    elapsed.as_secs_f64() * 1e6 / calls
}

/// Microseconds per query for each of `RUNS` runs of `work`, which runs `queries` queries.
/// Each run repeats `work` for about 50 ms.
fn measure(queries: usize, mut work: impl FnMut()) -> Vec<f64> {
    let reps = reps(&mut work);
    let mut times: Vec<f64> = (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..reps {
                work();
            }
            micros(start.elapsed(), (reps * queries) as f64)
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times
}

fn median(sorted: &[f64]) -> f64 {
    sorted[sorted.len() / 2]
}

fn spread(sorted: &[f64]) -> String {
    format!(
        "{:.2} ({:.2}–{:.2})",
        median(sorted),
        sorted[0],
        sorted[sorted.len() - 1]
    )
}
