use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};

use rune::Waker;

pub(crate) struct OriginalWrite {
    result: mpsc::Receiver<io::Result<PathBuf>>,
    worker: Option<JoinHandle<()>>,
}

impl OriginalWrite {
    pub(crate) fn start(path: PathBuf, bytes: Arc<[u8]>, wake: Option<Waker>) -> io::Result<Self> {
        let (send, result) = mpsc::channel();
        let worker = thread::Builder::new().spawn(move || {
            let written = write_original(&path, &bytes).map(|()| path);
            let _ = send.send(written);
            if let Some(wake) = wake {
                wake();
            }
        })?;
        let mut write = Self {
            result,
            worker: Some(worker),
        };
        if rune::quitting() {
            write.finish();
        }
        Ok(write)
    }

    fn finish(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    pub(crate) fn take(&self) -> Option<io::Result<PathBuf>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err(io::Error::other("the writer stopped")))
            }
        }
    }
}

impl Drop for OriginalWrite {
    fn drop(&mut self) {
        self.finish();
    }
}

fn write_original(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let identifier = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = path.with_file_name(format!(".prints-{}-{identifier}.tmp", std::process::id()));
    let mut file = File::options()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let written = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    let installed = written.and_then(|_| std::fs::rename(&temporary, path));
    if installed.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    installed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_accepted_write_finishes_when_its_owner_closes() {
        let path = std::env::temp_dir().join(format!("prints-write-test-{}", std::process::id()));
        let bytes = Arc::<[u8]>::from(&b"approved export"[..]);
        let job = OriginalWrite::start(path.clone(), bytes.clone(), None).unwrap();
        drop(job);
        let written = std::fs::read(&path);
        std::fs::remove_file(path).unwrap();
        assert_eq!(written.unwrap(), bytes.as_ref());
    }
}
