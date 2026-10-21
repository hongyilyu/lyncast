//! Nothing here goes through GPUI's image loader, which reads a path with `fs::read` and so waits
//! on a FIFO.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{AppContext, Context, RenderImage, Task, swap_rgba_pa_to_bgra};
use image::{Limits, RgbaImage};
use resvg::{tiny_skia, usvg};

use super::pixels::{self, Bgra};
use crate::files;
use crate::vars::Vars;

/// How many icons the cache keeps, loaded or not.
pub const CAPACITY: usize = 128;
/// The most pixels on an icon's longer side. Rows draw icons 24 logical pixels wide, so 48
/// covers an output at twice the scale.
const PIXELS: u32 = 48;
/// The most bytes read from one icon file.
const MAX_BYTES: u64 = 1 << 20;
/// The most pixels on either side of a raster icon that is decoded.
const MAX_SIDE: u32 = 1024;

const SIZES: [&str; 9] = [
    "48x48", "scalable", "64x64", "128x128", "256x256", "512x512", "32x32", "24x24", "16x16",
];

/// An app's `Icon=` value: an icon name to look up, or an absolute path to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconKey {
    Name(Arc<str>),
    Path(Arc<Path>),
}

/// The key for an `Icon=` value. An absolute path names the file, and anything else names an
/// icon.
pub fn key(value: &str) -> IconKey {
    if value.starts_with('/') {
        IconKey::Path(Path::new(value).into())
    } else {
        IconKey::Name(value.into())
    }
}

/// Where icon names are looked up: the hicolor theme under each of `themes`, then each of
/// `pixmaps`.
#[derive(Default)]
pub struct IconRoots {
    themes: Vec<PathBuf>,
    pixmaps: Vec<PathBuf>,
}

impl IconRoots {
    /// `$XDG_DATA_HOME/icons`, `$HOME/.icons`, and each `XDG_DATA_DIRS` entry's `icons`, then
    /// each entry's `pixmaps`, from the data directories the app scan reads.
    pub fn from_vars(vars: &Vars) -> IconRoots {
        let under =
            |dir: &str, name: &str| PathBuf::from(format!("{}/{name}", dir.trim_end_matches('/')));
        let dirs = vars.data_dirs();
        let home = vars.home.as_deref().filter(|home| home.starts_with('/'));
        let themes = vars
            .data_home()
            .map(|dir| under(&dir, "icons"))
            .into_iter()
            .chain(home.map(|home| under(home, ".icons")))
            .chain(dirs.iter().map(|dir| under(dir, "icons")))
            .collect();
        let pixmaps = dirs.iter().map(|dir| under(dir, "pixmaps")).collect();
        IconRoots { themes, pixmaps }
    }
}

/// The decoded icons, most recently asked for first.
pub struct Icons {
    roots: Arc<IconRoots>,
    slots: Vec<(IconKey, Slot)>,
    loading: Option<Task<()>>,
}

enum Slot {
    Wanted,
    Loading,
    Ready(Arc<RenderImage>),
    Missing,
}

impl Icons {
    pub fn new(roots: IconRoots) -> Icons {
        Icons {
            roots: Arc::new(roots),
            slots: Vec::new(),
            loading: None,
        }
    }

    /// The decoded icon, or `None` while it loads and when it has none. The first ask for a
    /// key queues its load. An ask past [`CAPACITY`] drops the least recent icon, and its image
    /// leaves every window's atlas.
    pub fn get(&mut self, key: &IconKey, cx: &mut Context<Icons>) -> Option<Arc<RenderImage>> {
        let slot = match self.slots.iter().position(|(held, _)| held == key) {
            Some(at) => self.slots.remove(at),
            None => (key.clone(), Slot::Wanted),
        };
        let image = match &slot.1 {
            Slot::Ready(image) => Some(Arc::clone(image)),
            Slot::Wanted | Slot::Loading | Slot::Missing => None,
        };
        self.slots.insert(0, slot);
        if self.slots.len() > CAPACITY
            && let Some((_, Slot::Ready(image))) = self.slots.pop()
        {
            // Rows ask while their window paints, and drop_image skips the window that paints.
            cx.defer(move |cx| cx.drop_image(image, None));
        }
        self.pump(cx);
        image
    }

    /// How many icons the cache holds, loaded or not.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// The decoded icon for `key`, if the cache holds one, without asking for it.
    #[cfg(test)]
    pub fn peek(&self, key: &IconKey) -> Option<Arc<RenderImage>> {
        self.slots.iter().find_map(|(held, slot)| match slot {
            Slot::Ready(image) if held == key => Some(Arc::clone(image)),
            _ => None,
        })
    }

    fn pump(&mut self, cx: &mut Context<Icons>) {
        if self.loading.is_some() {
            return;
        }
        let Some((key, slot)) = self
            .slots
            .iter_mut()
            .find(|(_, slot)| matches!(slot, Slot::Wanted))
        else {
            return;
        };
        *slot = Slot::Loading;
        let (key, roots) = (key.clone(), Arc::clone(&self.roots));
        self.loading = Some(cx.spawn(async move |icons, cx| {
            let wanted = key.clone();
            let loaded = cx
                .background_spawn(async move { load(&wanted, &roots) })
                .await;
            let _ = icons.update(cx, |icons, cx| icons.land(&key, loaded, cx));
        }));
    }

    fn land(&mut self, key: &IconKey, loaded: Option<RenderImage>, cx: &mut Context<Icons>) {
        self.loading = None;
        if let Some((_, slot)) = self.slots.iter_mut().find(|(held, _)| held == key) {
            *slot = match loaded {
                Some(image) => Slot::Ready(Arc::new(image)),
                None => Slot::Missing,
            };
            cx.notify();
        }
        self.pump(cx);
    }
}

fn load(key: &IconKey, roots: &IconRoots) -> Option<RenderImage> {
    let (_, pixels) = find(key, roots)?;
    Some(pixels.into_render_image())
}

fn find(key: &IconKey, roots: &IconRoots) -> Option<(PathBuf, Bgra)> {
    candidates(key, roots).into_iter().find_map(|path| {
        let bytes = files::read_regular(&path, MAX_BYTES).ok()?;
        let pixels = decode(&path, &bytes)?;
        Some((path, pixels))
    })
}

fn candidates(key: &IconKey, roots: &IconRoots) -> Vec<PathBuf> {
    let name = match key {
        IconKey::Path(path) => return vec![path.to_path_buf()],
        IconKey::Name(name) => name,
    };
    let mut files = Vec::new();
    for size in SIZES {
        let file = match size {
            "scalable" => format!("{name}.svg"),
            _ => format!("{name}.png"),
        };
        for root in &roots.themes {
            files.push(root.join("hicolor").join(size).join("apps").join(&file));
        }
    }
    for root in &roots.pixmaps {
        files.push(root.join(format!("{name}.png")));
        files.push(root.join(format!("{name}.svg")));
    }
    files
}

fn decode(path: &Path, bytes: &[u8]) -> Option<Bgra> {
    let pixels = match path.extension() {
        Some(extension) if extension == "svg" => svg(bytes)?,
        _ => raster(bytes)?,
    };
    let drawn = pixels.0.pixels().any(|pixel| pixel[3] > 0);
    drawn.then_some(pixels)
}

fn raster(bytes: &[u8]) -> Option<Bgra> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    pixels::decode_fitted(bytes, limits, (PIXELS, PIXELS)).ok()
}

fn svg(bytes: &[u8]) -> Option<Bgra> {
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(bytes, &options).ok()?;
    let size = tree.size();
    let scale = PIXELS as f32 / size.width().max(size.height());
    let (width, height) = fit(size.width() * scale, size.height() * scale);
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut data = pixmap.take();
    for pixel in data.as_chunks_mut::<4>().0 {
        swap_rgba_pa_to_bgra(pixel);
    }
    RgbaImage::from_raw(width, height, data).map(Bgra)
}

fn fit(width: f32, height: f32) -> (u32, u32) {
    let scale = (PIXELS as f32 / width.max(height)).min(1.0);
    let side = |length: f32| ((length * scale).round() as u32).max(1);
    (side(width), side(height))
}

#[cfg(test)]
pub(super) mod tests {
    use std::env;
    use std::ffi::OsString;
    use std::fs::{self, File};
    use std::io::{Cursor, Write};
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::{Path, PathBuf};
    use std::process;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread::{self, JoinHandle};

    use image::{ImageFormat, Rgba, RgbaImage};

    use super::{IconRoots, find, key};
    use crate::doc::{self, Block};
    use crate::palette::tests::mkfifo;
    use crate::vars::Vars;

    #[test]
    fn every_icon_loads_as_the_doc_says() {
        let fixtures = doc::PALETTE
            .read(&doc::PALETTE.text())
            .unwrap_or_else(|error| panic!("palette.md: {error}"));
        let of = |kind: &str| -> Vec<&Block> {
            fixtures.iter().filter(|block| block.kind == kind).collect()
        };
        let tree = env::current_exe()
            .expect("the test binary's path")
            .with_file_name("palette-icons")
            .join(process::id().to_string());
        let _ = fs::remove_dir_all(&tree);
        let writers: Vec<Writer> = of("icon")
            .into_iter()
            .filter_map(|block| make(block, &tree))
            .map(Writer::start)
            .collect();
        let roots = IconRoots::from_vars(&vars(&of("env"), &tree));
        let [lines] = of("icons")[..] else {
            panic!("palette.md holds one icons block");
        };
        let mut failures = Vec::new();
        for line in &lines.lines {
            let (value, want) = line
                .split_once(" =>")
                .unwrap_or_else(|| panic!("line {}: not <Icon=> =>: {line}", lines.line));
            let value = value.replace("$TREE", tree.to_str().expect("a UTF-8 tree"));
            let got = match find(&key(&value), &roots) {
                Some((path, pixels)) => {
                    let path = path.strip_prefix(&tree).expect("a file in the tree");
                    let (width, height) = pixels.0.dimensions();
                    format!(" {} {width}x{height}", path.display())
                }
                None => String::new(),
            };
            if got != want {
                failures.push(format!(
                    "line {}: {line}\n  got {value} =>{got}",
                    lines.line
                ));
            }
        }
        for writer in writers {
            if let Some(opened) = writer.finish() {
                failures.push(format!("{opened:?}: a load opened the FIFO"));
            }
        }
        assert!(
            failures.is_empty(),
            "{} palette.md lines fail:\n{}",
            failures.len(),
            failures.join("\n")
        );
        fs::remove_dir_all(&tree).expect("the tree goes");
    }

    /// An opaque PNG `width` by `height`.
    pub fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        RgbaImage::from_pixel(width, height, Rgba([0xe6, 0x60, 0x00, 0xff]))
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .expect("the PNG encodes");
        bytes
    }

    fn make(block: &Block, tree: &Path) -> Option<PathBuf> {
        let [relative] = block.args.as_slice() else {
            panic!("line {}: an icon block names one path", block.line);
        };
        let path = tree.join(relative);
        fs::create_dir_all(path.parent().expect("a path under the tree")).unwrap();
        let words: Vec<&str> = match block.lines.as_slice() {
            [line] => line.split(' ').collect(),
            _ => Vec::new(),
        };
        let bytes = match words.as_slice() {
            ["fifo"] => {
                mkfifo(&path).expect("the icon FIFO");
                return Some(path);
            }
            ["png", size, padded @ ..] => {
                let (width, height) = size.split_once('x').expect("png <w>x<h>");
                let mut bytes = png(width.parse().unwrap(), height.parse().unwrap());
                if let [padded] = padded {
                    bytes.resize(padded.parse().expect("png <w>x<h> <bytes>"), 0);
                }
                bytes
            }
            _ => {
                let text = format!("{}\n", block.lines.join("\n"));
                text.replace("$TREE", tree.to_str().unwrap()).into_bytes()
            }
        };
        fs::write(&path, bytes).unwrap();
        None
    }

    fn vars(env: &[&Block], tree: &Path) -> Vars {
        let [block] = env else {
            panic!("palette.md holds one env block");
        };
        let var = |name: &str| {
            block.lines.iter().find_map(|line| {
                let value = line.strip_prefix(name)?.strip_prefix('=')?;
                Some(value.replace("$TREE", tree.to_str().unwrap()))
            })
        };
        Vars {
            data_home: var("XDG_DATA_HOME"),
            data_dirs: var("XDG_DATA_DIRS").map(OsString::from),
            home: var("HOME"),
            ..Vars::default()
        }
    }

    struct Writer {
        fifo: PathBuf,
        opens: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
        thread: JoinHandle<()>,
    }

    impl Writer {
        fn start(fifo: PathBuf) -> Writer {
            let (opens, stop) = (
                Arc::new(AtomicUsize::new(0)),
                Arc::new(AtomicBool::new(false)),
            );
            let (path, opened, stopped) = (fifo.clone(), Arc::clone(&opens), Arc::clone(&stop));
            let thread = thread::spawn(move || {
                while !stopped.load(Ordering::SeqCst) {
                    let mut file = File::options()
                        .write(true)
                        .open(&path)
                        .expect("the FIFO opens");
                    opened.fetch_add(1, Ordering::SeqCst);
                    let _ = file.write_all(b"x");
                }
            });
            Writer {
                fifo,
                opens,
                stop,
                thread,
            }
        }

        fn finish(self) -> Option<PathBuf> {
            let opened = self.opens.load(Ordering::SeqCst) > 0;
            self.stop.store(true, Ordering::SeqCst);
            // A reader that does not wait for a writer, which lets the writer's open return.
            let reader = File::options()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&self.fifo)
                .expect("the FIFO opens to read");
            self.thread.join().expect("the writer ends");
            drop(reader);
            opened.then_some(self.fifo)
        }
    }
}
