use std::path::PathBuf;
use thiserror::Error;

/// Why the static data could not be downloaded, extracted, or read back from disk. Each case names
/// the file it is about, since the archive holds a hundred and the data directory four.
#[derive(Debug, Error)]
pub enum StaticDataError {
    #[error("downloading the static data archive failed: {0}")]
    Download(#[from] reqwest::Error),

    #[error("the static data archive could not be opened: {0}")]
    Archive(#[from] zip::result::ZipError),

    #[error("{file} in the static data archive: {source}")]
    ArchiveFile {
        file: &'static str,
        source: zip::result::ZipError,
    },

    #[error("{file} in the static data archive, line {line}: {source}")]
    Record {
        file: &'static str,
        line: usize,
        source: serde_json::Error,
    },

    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("{}: {source}", path.display())]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
}
