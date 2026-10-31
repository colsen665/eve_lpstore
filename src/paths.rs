//! Where this program keeps its files, per platform.
//!
//! Both directories used to be relative to the working directory, so running the binary from
//! anywhere else silently started an empty cache: the static data archive fetched again, every ESI
//! response fetched again. These roots are absolute and the same wherever the program is invoked
//! from.
//!
//! Neither is Roaming on Windows. `cache_dir()` is the Local one, which keeps 4,584 regenerable
//! response files off a domain profile, and `data_local_dir()` is Local too, so the two agree. On
//! Linux and macOS the two differ by location only, `~/.cache/eve_lpstore` and
//! `~/.local/share/eve_lpstore`.

use std::path::PathBuf;

/// Reverse-domain qualifier, organization, application. Both earlier fields are empty on purpose,
/// so Linux gets `~/.cache/eve_lpstore` rather than `~/.cache/com.example.eve_lpstore`. Changing
/// these strings orphans whatever the previous ones pointed at, so they are decided here once.
const QUALIFIER: &str = "";
const ORGANIZATION: &str = "";
const APPLICATION: &str = "eve_lpstore";

/// The two roots the program writes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// ESI responses, keyed by endpoint and expiry. Regenerable, and the larger of the two by far.
    pub cache: PathBuf,
    /// The four extracted static data files. Regenerable, but the fetch is the whole EVE Online
    /// data archive, so it is the more expensive one to lose.
    pub data: PathBuf,
    /// Settings the user chose rather than derived data, which is where `filters.json` belongs.
    /// Kept apart from the two above so clearing a cache never takes the remembered filters with
    /// it, and so a data reset leaves them alone.
    pub config: PathBuf,
}

impl Paths {
    /// Find the platform locations.
    ///
    /// `None` when the operating system reports no home directory, which leaves nowhere to put
    /// either root. Callers refuse to start on that rather than falling back to the working
    /// directory, since the fallback is the behaviour this module exists to remove.
    pub fn discover() -> Option<Self> {
        let project = directories::ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)?;
        Some(Self {
            cache: project.cache_dir().to_path_buf(),
            data: project.data_local_dir().to_path_buf(),
            config: project.config_dir().to_path_buf(),
        })
    }

    /// Point the ESI cache at `dir` when given, leaving the static data where the platform puts
    /// it. This is what `--cache-dir` means: it names a cache, and the data files are not a cache
    /// in that sense even though they are regenerable.
    #[must_use]
    pub fn with_cache_override(self, dir: Option<&str>) -> Self {
        match dir {
            Some(dir) if !dir.trim().is_empty() => Self {
                cache: PathBuf::from(dir),
                ..self
            },
            _ => self,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The platform roots, or the ones a test can write to when there is no home directory.
    fn discovered() -> Paths {
        Paths::discover().expect("the test environment has no home directory")
    }

    #[test]
    fn both_roots_are_absolute() {
        let paths = discovered();
        assert!(paths.cache.is_absolute(), "{}", paths.cache.display());
        assert!(paths.data.is_absolute(), "{}", paths.data.display());
    }

    #[test]
    fn neither_root_is_the_working_directory() {
        // The bug these paths fix was `cache/` and `data/` under whatever directory the binary
        // happened to run from.
        let paths = discovered();
        let here = std::env::current_dir().unwrap();
        for root in [&paths.cache, &paths.data] {
            // Only the location is checked. On Windows the roots do end in `cache` and `data`,
            // under `%LOCALAPPDATA%\eve_lpstore`.
            assert!(
                !root.starts_with(&here),
                "{root:?} is the working directory {here:?} or inside it"
            );
        }
    }

    #[test]
    fn the_cache_and_the_data_are_separate_roots() {
        let paths = discovered();
        assert_ne!(paths.cache, paths.data);
    }

    #[test]
    fn the_override_moves_the_cache_only() {
        let paths = discovered().with_cache_override(Some("/tmp/other-cache"));
        assert_eq!(paths.cache, PathBuf::from("/tmp/other-cache"));
        assert_eq!(
            paths.data,
            discovered().data,
            "the data root is not a cache"
        );
    }

    #[test]
    fn a_blank_override_keeps_the_platform_cache() {
        for blank in [None, Some(""), Some("   ")] {
            assert_eq!(discovered().with_cache_override(blank), discovered());
        }
    }
}
