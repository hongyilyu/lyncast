//! Nothing here goes through GPUI's image loader, which reads a path with `fs::read` and so waits
//! on a FIFO.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{AppContext, Context, RenderImage, Task, Window};
use image::{ImageReader, Limits};
use lyncast_core::ClipId;

use super::pixels;
use crate::clip::{Content, Payload, Row};
use crate::files;

/// The most pixels an image may have and still decode.
pub const MAX_PIXELS: u64 = 32_000_000;
const RGBA16_BYTES: u64 = 8;
/// The most bytes read from the image file a file clip names, as many as a clip holds.
pub const MAX_FILE_BYTES: u64 = 25 << 20;
/// The pane at twice the scale, in device pixels. A larger image is scaled down to fit before it
/// is drawn, so the pane holds at most this many pixels.
pub const FIT: (u32, u32) = (840, 480);
const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// What the pane shows of the selected clip.
pub enum Shown {
    /// A text: its preview, which the view reads from the row.
    Text,
    /// A file list whose first file is no image: its paths.
    Files,
    Loading,
    /// A decoded image, and the size it had before it was scaled down.
    Image {
        image: Arc<RenderImage>,
        width: u32,
        height: u32,
    },
    /// An image over [`MAX_PIXELS`], which is not decoded.
    TooLarge {
        width: u32,
        height: u32,
    },
    /// Why the bytes did not read or decode.
    Unreadable(String),
}

/// What the pane does for a clip.
pub enum What {
    Text,
    Files,
    /// Loads an image from here, or shows why it cannot.
    Load(Result<Source, String>),
}

/// Where an image's bytes are.
pub enum Source {
    /// A clip's own bytes, read as a copy reads them.
    Clip(Payload),
    /// The file a file clip names first.
    File(PathBuf),
}

/// What the pane does for `row`. `payload` gives the bytes of an image row.
pub fn what(row: &Row, payload: impl FnOnce() -> Result<Payload, String>) -> What {
    match row.content {
        Content::Text(_) => What::Text,
        Content::Image => What::Load(payload().map(Source::Clip)),
        Content::Files => {
            let first = row
                .preview
                .as_deref()
                .and_then(|paths| paths.lines().next());
            match first.filter(|path| is_image(Path::new(path))) {
                Some(path) => What::Load(Ok(Source::File(PathBuf::from(path)))),
                None => What::Files,
            }
        }
    }
}

/// The palette's one preview, which every window draws, and its one load. It outlives the
/// windows, so the load out as a window closes is the one load out for the next window too.
#[derive(Default)]
pub struct Preview {
    slot: Option<(ClipId, Shown)>,
    out: Option<(Arc<Out>, Task<()>)>,
    wanted: Option<(ClipId, Source)>,
    #[cfg(test)]
    counts: Counts,
}

#[derive(Default)]
struct Out {
    stop: AtomicBool,
    #[cfg(test)]
    decoded: AtomicBool,
}

enum Loaded {
    Shown(Shown),
    Stopped,
}

/// How the loads went, for the Tier 3 tests: how many started, how many stopped, how many
/// decoded, and the most out at once.
#[cfg(test)]
#[derive(Clone, Copy, Default)]
pub struct Counts {
    pub started: usize,
    pub stopped: usize,
    pub decoded: usize,
    pub most_out: usize,
    out: usize,
}

#[cfg(test)]
impl Counts {
    fn started(&mut self) {
        self.started += 1;
        self.out += 1;
        self.most_out = self.most_out.max(self.out);
    }

    fn landed(&mut self, loaded: &Loaded, out: Option<&(Arc<Out>, Task<()>)>) {
        self.out -= 1;
        self.stopped += usize::from(matches!(loaded, Loaded::Stopped));
        self.decoded +=
            usize::from(out.is_some_and(|(out, _)| out.decoded.load(Ordering::Acquire)));
    }
}

impl Preview {
    /// The clip the pane shows.
    pub fn of(&self) -> Option<ClipId> {
        self.slot.as_ref().map(|(id, _)| *id)
    }

    pub fn shown(&self) -> Option<&Shown> {
        self.slot.as_ref().map(|(_, shown)| shown)
    }

    #[cfg(test)]
    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// Shows clip `id`, which `what` says how, or nothing. The old image leaves every window's
    /// atlas. One load is out at a time: the load out stops before it decodes unless it is past
    /// that already, and the newest selected image waits for it to land.
    pub fn select(
        &mut self,
        id: Option<ClipId>,
        what: Option<What>,
        window: &mut Window,
        cx: &mut Context<Preview>,
    ) {
        if let Some((_, Shown::Image { image, .. })) = self.slot.take() {
            cx.drop_image(image, Some(window));
        }
        if let Some((out, _)) = &self.out {
            out.stop.store(true, Ordering::Release);
        }
        self.wanted = None;
        let (Some(id), Some(what)) = (id, what) else {
            return;
        };
        let shown = match what {
            What::Text => Shown::Text,
            What::Files => Shown::Files,
            What::Load(Err(reason)) => Shown::Unreadable(reason),
            What::Load(Ok(source)) => {
                match self.out {
                    Some(_) => self.wanted = Some((id, source)),
                    None => self.start(id, source, cx),
                }
                Shown::Loading
            }
        };
        self.slot = Some((id, shown));
    }

    fn land(&mut self, id: ClipId, loaded: Loaded, cx: &mut Context<Preview>) {
        #[cfg(test)]
        self.counts.landed(&loaded, self.out.as_ref());
        self.out = None;
        if let (Some((of, held @ Shown::Loading)), Loaded::Shown(shown)) = (&mut self.slot, loaded)
            && *of == id
        {
            *held = shown;
            self.wanted = None;
        }
        if let Some((id, source)) = self.wanted.take() {
            self.start(id, source, cx);
        }
        cx.notify();
    }

    fn start(&mut self, id: ClipId, source: Source, cx: &mut Context<Preview>) {
        let out = Arc::new(Out::default());
        let shared = Arc::clone(&out);
        let task = cx.spawn(async move |preview, cx| {
            let loaded = cx
                .background_spawn(async move { load(source, &shared) })
                .await;
            let _ = preview.update(cx, |preview, cx| preview.land(id, loaded, cx));
        });
        self.out = Some((out, task));
        #[cfg(test)]
        self.counts.started();
    }
}

fn load(source: Source, out: &Out) -> Loaded {
    let decoded = match source {
        Source::Clip(payload) => payload.blocking_read_with(|bytes| decode(bytes, out)),
        Source::File(path) => match files::read_regular(&path, MAX_FILE_BYTES) {
            Ok(bytes) => Ok(decode(&bytes, out)),
            Err(unread) => Err(format!("{}: {unread}", path.display())),
        },
    };
    decoded.unwrap_or_else(|reason| Loaded::Shown(Shown::Unreadable(reason)))
}

/// The image in `bytes`, by its content, scaled down to fit [`FIT`], unless `out` was stopped
/// once its size was read.
fn decode(bytes: &[u8], out: &Out) -> Loaded {
    let size = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .expect("reading a slice cannot fail")
        .into_dimensions();
    let (width, height) = match size {
        Ok(size) => size,
        Err(error) => return Loaded::Shown(Shown::Unreadable(error.to_string())),
    };
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Loaded::Shown(Shown::TooLarge { width, height });
    }
    if out.stop.load(Ordering::Acquire) {
        return Loaded::Stopped;
    }
    #[cfg(test)]
    out.decoded.store(true, Ordering::Release);
    let mut limits = Limits::default();
    limits.max_image_width = Some(width);
    limits.max_image_height = Some(height);
    limits.max_alloc = Some(MAX_PIXELS * RGBA16_BYTES);
    Loaded::Shown(match pixels::decode_fitted(bytes, limits, FIT) {
        Ok(pixels) => Shown::Image {
            image: Arc::new(pixels.into_render_image()),
            width,
            height,
        },
        Err(reason) => Shown::Unreadable(reason),
    })
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            IMAGE_EXTENSIONS
                .iter()
                .any(|image| extension.eq_ignore_ascii_case(image))
        })
}
