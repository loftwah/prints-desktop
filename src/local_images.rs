use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rune::{Image, ImageCache, Latest, Waker};

use crate::{OriginalError, Picture, read_original};

/// Keep one contact sheet page and the selected originals in independent lanes.
/// Each lane issues one read and decode at a time, even during rapid navigation.
#[derive(Default)]
pub(super) struct LocalImages {
    originals: Lane<OriginalImage>,
    thumbnails: Lane<Image>,
}

impl LocalImages {
    pub(super) fn set_waker(&mut self, wake: Waker) {
        self.originals.request.set_waker(wake.clone());
        self.thumbnails.request.set_waker(wake);
    }

    pub(super) fn poll(
        &mut self,
        originals: &[PathBuf],
        thumbnails: &[PathBuf],
        cache: &Arc<ImageCache>,
    ) -> [Option<PathBuf>; 2] {
        [
            self.originals.poll(originals, cache, |path, cache| {
                let original = read_original(path)?;
                let digest = blake3::hash(&original.bytes);
                let picture = Picture::new(original.bytes, cache);
                let _ = picture.image.wait();
                Ok(OriginalImage {
                    picture,
                    digest,
                    filename: original.filename,
                })
            }),
            self.thumbnails.poll(thumbnails, cache, |path, cache| {
                let original = read_original(path)?;
                let image = cache.load_scaled(original.bytes, 192.0, 2.0);
                let _ = image.wait();
                Ok(image)
            }),
        ]
    }

    pub(super) fn original(&self, path: &Path) -> Option<&OriginalImage> {
        self.originals.items.get(path)?.as_ref().ok()
    }

    pub(super) fn thumbnail(&self, path: &Path) -> Option<&Image> {
        self.thumbnails.items.get(path)?.as_ref().ok()
    }

    pub(super) fn available(&self, path: &Path) -> Option<bool> {
        self.originals
            .items
            .get(path)
            .map(Result::is_ok)
            .or_else(|| self.thumbnails.items.get(path).map(Result::is_ok))
    }

    pub(super) fn invalidate(&mut self, path: &Path) {
        self.originals.invalidate(path);
        self.thumbnails.invalidate(path);
    }
}

pub(super) struct OriginalImage {
    pub picture: Picture,
    pub digest: blake3::Hash,
    pub filename: String,
}

struct Lane<T: Send + 'static> {
    items: HashMap<PathBuf, Result<T, OriginalError>>,
    pending: Option<PathBuf>,
    request: Latest<Result<T, OriginalError>>,
}

impl<T: Send + 'static> Default for Lane<T> {
    fn default() -> Self {
        Self {
            items: HashMap::new(),
            pending: None,
            request: Latest::default(),
        }
    }
}

impl<T: Send + 'static> Lane<T> {
    fn poll(
        &mut self,
        wanted: &[PathBuf],
        cache: &Arc<ImageCache>,
        load: fn(&Path, &ImageCache) -> Result<T, OriginalError>,
    ) -> Option<PathBuf> {
        self.items.retain(|path, _| wanted.contains(path));
        let mut ready = None;
        if let Some(result) = self.request.take()
            && let Some(path) = self.pending.take()
            && wanted.contains(&path)
        {
            self.items.insert(path.clone(), result);
            ready = Some(path);
        }
        if self.pending.is_none()
            && let Some(path) = wanted.iter().find(|path| !self.items.contains_key(*path))
        {
            self.pending = Some(path.clone());
            self.request
                .request((path.clone(), cache.clone()), move |(path, cache)| {
                    load(&path, &cache)
                });
        }
        ready
    }

    fn invalidate(&mut self, path: &Path) {
        self.items.remove(path);
        if self.pending.as_deref() == Some(path) {
            self.pending = None;
            self.request.cancel();
        }
    }
}
