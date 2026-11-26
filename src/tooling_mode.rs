use std::ffi::OsString;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rune::prefs::Prefs;

const MARKER: &str = ".prints-verification-profile";
const MARKER_CONTENTS: &[u8] = b"Prints verification workspace 1\n";

enum Mode {
    Live,
    Temporary,
    Profile(PathBuf),
}

static MODE: OnceLock<Mode> = OnceLock::new();

pub(crate) fn prepare(arguments: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let mut arguments = arguments.into_iter().peekable();
    let mut explicit = None;
    let mut tooling = false;
    while let Some(argument) = arguments.next() {
        if argument == "--verify-profile" {
            let path = arguments
                .next_if(|value| !value.to_string_lossy().starts_with('-'))
                .ok_or("--verify-profile needs a dedicated directory")?;
            if explicit.replace(PathBuf::from(path)).is_some() {
                return Err("Specify one verification profile directory.".into());
            }
        } else if !argument.to_string_lossy().starts_with("-psn_") {
            // A Rune option or positional argument must never load the real profile.
            // New tooling flags therefore default to isolation as well.
            tooling = true;
        }
    }
    let mode = if let Some(directory) = explicit {
        Mode::Profile(verification_profile(&directory).map_err(|error| error.to_string())?)
    } else if tooling {
        #[cfg(debug_assertions)]
        if let Some(path) = std::env::var_os("PRINTS_REPLAY_PREFS") {
            let _ = MODE.set(Mode::Profile(PathBuf::from(path)));
            return Ok(());
        }
        Mode::Temporary
    } else {
        Mode::Live
    };
    let _ = MODE.set(mode);
    Ok(())
}

pub(crate) fn isolated() -> bool {
    matches!(MODE.get(), Some(Mode::Temporary | Mode::Profile(_)))
}

pub(crate) fn preferences(app_id: &str) -> Prefs {
    match MODE.get() {
        Some(Mode::Temporary) => Prefs::memory(),
        Some(Mode::Profile(path)) => Prefs::at(path),
        _ => Prefs::open(app_id),
    }
}

fn verification_profile(directory: &Path) -> io::Result<PathBuf> {
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = fs::symlink_metadata(directory)?;
    if !metadata.is_dir() {
        return Err(io::Error::other("Use a dedicated verification directory."));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::other(
                "The verification directory must be private.",
            ));
        }
    }
    let directory = fs::canonicalize(directory)?;
    let marker = directory.join(MARKER);
    match fs::symlink_metadata(&marker) {
        Ok(metadata)
            if metadata.is_file()
                && metadata.len() == MARKER_CONTENTS.len() as u64
                && fs::read(&marker)? == MARKER_CONTENTS => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if fs::read_dir(&directory)?.next().is_some() {
                return Err(io::Error::other("Choose an empty verification directory."));
            }
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&marker)?
                .write_all(MARKER_CONTENTS)?;
        }
        _ => {
            return Err(io::Error::other(
                "The verification directory has an invalid marker.",
            ));
        }
    }
    // Prefs can rename a corrupt file. Ensure the only paths it can open in this
    // mode are files belonging to the explicitly selected verification directory.
    for name in ["prefs.json", "prefs.draft-workspace.json"] {
        match fs::symlink_metadata(directory.join(name)) {
            Ok(metadata) if metadata.is_file() => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            _ => {
                return Err(io::Error::other(
                    "The verification profile contains an invalid file.",
                ));
            }
        }
    }
    Ok(directory.join("prefs.json"))
}
