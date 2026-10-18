mod common;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use image::RgbImage;

use common::clipboard::Helpers;
use common::doc::{self, Block};
use common::{OK, Ran, World, poll, run_bounded, session_later};

const TREES: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/palette-trees");
const SHOTS: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/palette-shots");

#[test]
fn every_summon_block_runs_as_the_doc_says() {
    let fixtures = doc::PALETTE
        .read(&doc::PALETTE.text())
        .unwrap_or_else(|error| panic!("palette.md: {error}"));
    let blocks: Vec<&Block> = fixtures
        .iter()
        .filter(|block| block.kind == "summon")
        .collect();
    assert_eq!(
        blocks.len(),
        13,
        "palette.md holds 13 summon blocks, so a lost block fails"
    );
    let trees = Path::new(TREES);
    match fs::remove_dir_all(trees) {
        Err(error) if error.kind() != ErrorKind::NotFound => panic!("{trees:?}: {error}"),
        _ => {}
    }
    let failures: Vec<String> = blocks
        .into_iter()
        .flat_map(|block| run(&fixtures, block, trees.join(block.line.to_string())))
        .collect();
    assert!(
        failures.is_empty(),
        "{} palette.md lines fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn run(fixtures: &[Block], block: &Block, tree: PathBuf) -> Vec<String> {
    let world = World::compositor(fixtures, tree);
    let mut helpers = Helpers::new(&world);
    let keys = |line: &str| line.starts_with("$ keys ");
    let failures = session_later(
        &world,
        block,
        &mut |line, newest| {
            trace(&world, line)
                .or_else(|| palette(&world, line))
                .or_else(|| shot(&world, line))
                .or_else(|| helpers.run(line, newest))
        },
        &keys,
    );
    drop(helpers);
    if failures.is_empty() {
        world.passed();
    }
    failures
}

fn trace(world: &World, line: &str) -> Option<Ran> {
    let words: Vec<&str> = line.strip_prefix("$ ")?.split(' ').collect();
    let stdout = world.runtime.path().join("daemon.stdout");
    let read = || fs::read_to_string(&stdout).unwrap_or_default();
    Some(match words.as_slice() {
        ["wait-trace", event, seq] => {
            let want = format!("PROBE seq={seq} ev={event} ");
            match poll(|| {
                read()
                    .lines()
                    .any(|held| held.starts_with(&want))
                    .then_some(())
            }) {
                Some(()) => OK,
                None => (Vec::new(), read().into_bytes(), Some(1)),
            }
        }
        ["count-trace", event] => {
            let want = format!("ev={event}");
            let count = read()
                .lines()
                .filter(|held| held.split(' ').nth(2) == Some(want.as_str()))
                .count();
            (format!("{count}\n").into_bytes(), Vec::new(), Some(0))
        }
        _ => return None,
    })
}

const X11: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/x11.py");

fn palette(world: &World, line: &str) -> Option<Ran> {
    let words: Vec<&str> = line.strip_prefix("$ ")?.split(' ').collect();
    Some(match words.as_slice() {
        ["keys", specs @ ..] => {
            let display = fs::read_to_string(world.runtime.path().join("display"))
                .expect("test-env.sh writes the display");
            let typed = format!(
                "$ python3 -I {X11} {} focus keys {}",
                display.trim(),
                specs.join(" ")
            );
            run_bounded(world.command(&typed).0, &world.out)?
        }
        ["wait-summon", state] => {
            let want = format!("{state}\n");
            let mut last = Vec::new();
            let status = || run_bounded(world.command("$ lyncast status").0, &world.out);
            let waited = poll(|| {
                let (stdout, _, _) = status()?;
                if stdout == want.as_bytes() {
                    return Some(());
                }
                last = stdout;
                None
            });
            match waited {
                Some(()) => OK,
                None => (last, Vec::new(), Some(1)),
            }
        }
        _ => return None,
    })
}

fn shot(world: &World, line: &str) -> Option<Ran> {
    let words: Vec<&str> = line.strip_prefix("$ ")?.split(' ').collect();
    let ["shot", name, wanted @ ..] = &words[..] else {
        return None;
    };
    let file = world.runtime.path().join("Xvfb_screen0");
    let mut last = None;
    let mut seen = None;
    let shown = poll(|| {
        let screen = Screen::read(&file).ok()?;
        let lines = screen.palette();
        seen.clone_from(&lines);
        let lines = lines.filter(|lines| shows(lines, wanted));
        if lines.is_some() && lines == last {
            return Some((screen, lines?));
        }
        last = lines;
        None
    });
    let Some((screen, lines)) = shown else {
        let said = match seen {
            Some(seen) => format!("the screen never showed {}:\n{seen}", wanted.join(" ")),
            None => "no palette on the screen\n".to_owned(),
        };
        return Some((Vec::new(), said.into_bytes(), Some(1)));
    };
    let block = world.tree.file_name().expect("a tree named by its block");
    let png = Path::new(SHOTS).join(format!("{}-{name}.png", block.to_string_lossy()));
    fs::create_dir_all(SHOTS).expect("the shots directory");
    screen.png().save(&png).expect("the PNG saves");
    Some((lines.into_bytes(), Vec::new(), Some(0)))
}

fn shows(lines: &str, wanted: &[&str]) -> bool {
    let colors = lines.lines().find_map(|line| line.strip_prefix("colors "));
    let colors: Vec<&str> = colors.unwrap_or_default().split(' ').collect();
    wanted.iter().all(|color| colors.contains(color))
}

struct Screen {
    width: usize,
    height: usize,
    pixels: Vec<u32>,
}

impl Screen {
    /// Reads an XWD file of version 7 whose pixels are 32 bits, blue byte first, which is what
    /// Xvfb writes on this box. Its header is 25 numbers in either byte order, and the pixels
    /// start after the header and its colormap of 12-byte entries.
    fn read(path: &Path) -> Result<Screen, String> {
        let data = fs::read(path).map_err(|error| format!("{path:?}: {error}"))?;
        let word = |at: usize, big: bool| -> Option<usize> {
            let bytes: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
            let word = if big {
                u32::from_be_bytes(bytes)
            } else {
                u32::from_le_bytes(bytes)
            };
            usize::try_from(word).ok()
        };
        let big = match (word(4, true), word(4, false)) {
            (Some(7), _) => true,
            (_, Some(7)) => false,
            _ => return Err("not an XWD file of version 7".to_owned()),
        };
        let header: Vec<usize> = (0..25)
            .map(|at| word(at * 4, big))
            .collect::<Option<_>>()
            .ok_or("a short XWD header")?;
        let [size, _, _, _, width, height, _, order, .., bits, line] = header[..13] else {
            unreachable!("the header holds 25 numbers");
        };
        let (masks, colors) = ((header[14], header[15], header[16]), header[19]);
        if (bits, order, masks) != (32, 0, (0xff0000, 0xff00, 0xff)) {
            return Err(format!(
                "XWD pixels of {bits} bits, order {order}, masks {masks:x?}"
            ));
        }
        let start = size + colors * 12;
        if data.len() < start + height * line {
            return Err("the XWD file is shorter than its pixels".to_owned());
        }
        let pixels = (0..height)
            .flat_map(|y| (0..width).map(move |x| start + y * line + x * 4))
            .map(|at| u32::from_le_bytes([data[at], data[at + 1], data[at + 2], 0]))
            .collect();
        Ok(Screen {
            width,
            height,
            pixels,
        })
    }

    fn at(&self, x: usize, y: usize) -> u32 {
        self.pixels[y * self.width + x]
    }

    fn palette(&self) -> Option<String> {
        let background = self.pixels[0];
        let changed = (0..self.height)
            .flat_map(|y| (0..self.width).map(move |x| (x, y)))
            .filter(|&(x, y)| self.at(x, y) != background);
        let (mut left, mut top, mut right, mut bottom) = (usize::MAX, usize::MAX, 0, 0);
        for (x, y) in changed {
            (left, top) = (left.min(x), top.min(y));
            (right, bottom) = (right.max(x), bottom.max(y));
        }
        if left == usize::MAX {
            return None;
        }
        let (width, height) = (right + 1 - left, bottom + 1 - top);
        let inside = || (top..=bottom).flat_map(move |y| (left..=right).map(move |x| (x, y)));
        let edge: HashSet<u32> = inside()
            .filter(|&(x, y)| x == left || x == right || y == top || y == bottom)
            .map(|(x, y)| self.at(x, y))
            .collect();
        let edge = match edge.into_iter().collect::<Vec<_>>()[..] {
            [color] => format!("{color:06x}"),
            _ => "mixed".to_owned(),
        };
        let mut counts: HashMap<u32, usize> = HashMap::new();
        for (x, y) in inside() {
            *counts.entry(self.at(x, y)).or_default() += 1;
        }
        let mut colors: Vec<(u32, usize)> = counts
            .into_iter()
            .filter(|&(_, count)| count * 100 >= width * height)
            .collect();
        colors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let colors: Vec<String> = colors
            .iter()
            .map(|(color, _)| format!("{color:06x}"))
            .collect();
        Some(format!(
            "palette at {left},{top} {width}x{height}\nedge {edge}\ncolors {}\n",
            colors.join(" ")
        ))
    }

    fn png(&self) -> RgbImage {
        let bytes = self
            .pixels
            .iter()
            .flat_map(|pixel| {
                let [_, red, green, blue] = pixel.to_be_bytes();
                [red, green, blue]
            })
            .collect();
        let width = u32::try_from(self.width).expect("an XWD width is 32 bits");
        let height = u32::try_from(self.height).expect("an XWD height is 32 bits");
        RgbImage::from_raw(width, height, bytes).expect("a buffer the screen's size")
    }
}

#[test]
#[ignore = "the clipboard verbs run it as a helper"]
fn helper() {
    common::clipboard::helper();
}
