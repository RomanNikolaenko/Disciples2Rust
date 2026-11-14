//! Game updates from GitHub Releases.
//!
//! The release workflow publishes, for each `vX.Y.Z` tag, one package per platform
//! (`Disciples2Rust-vX.Y.Z-<target>.zip` / `.tar.gz`: the executable plus `assets/`) and a
//! `.sha256` file beside it. At startup a background thread asks GitHub for the latest
//! release; when it is newer than this build, the main menu's Update button lights up.
//!
//! Installing downloads this platform's package next to the game, checks its SHA-256,
//! unpacks it, and swaps in the new executable and `assets/`. The running executable is
//! renamed rather than overwritten (Windows allows that); old files keep an `.old`
//! suffix and are removed on the next start. A failed swap is rolled back. The game then
//! restarts into the new version.
//!
//! Only a packaged build (an `assets/` folder beside the executable) installs updates; a
//! development run (`cargo run`) only reports that one exists.

use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyhow::{Context as _, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/RomanNikolaenko/Disciples2Rust/releases/latest";
/// GitHub's API requires a User-Agent; a generic client name is enough.
const USER_AGENT: &str = concat!("Disciples2Rust/", env!("CARGO_PKG_VERSION"));
/// The release package target this build installs.
const TARGET: &str = if cfg!(all(windows, target_arch = "x86_64")) {
    "x86_64-pc-windows-msvc"
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    "x86_64-unknown-linux-gnu"
} else {
    ""
};
const PACKAGE_EXTENSION: &str = if cfg!(windows) { ".zip" } else { ".tar.gz" };
const CHECKSUM_EXTENSION: &str = ".sha256";
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Downloads larger than this are refused (packages are a few MB).
const MAX_PACKAGE_BYTES: u64 = 1 << 30;
/// Scratch folder for a download, beside the game.
const STAGING_DIR: &str = ".update";
const ASSETS_DIR: &str = "assets";
const OLD_SUFFIX: &str = ".old";

/// A release version (`major.minor.patch`), compared numerically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    major: u32,
    minor: u32,
    patch: u32,
}

impl Version {
    /// Parses `1.2.3` or `v1.2.3`; a pre-release or build suffix (`-beta`, `+meta`) is
    /// ignored.
    pub fn parse(text: &str) -> Option<Self> {
        let core = text.trim().trim_start_matches('v');
        let core = core.split(['-', '+']).next()?;
        let mut parts = core.split('.').map(|part| part.parse::<u32>().ok());
        let version = Self {
            major: parts.next()??,
            minor: parts.next()??,
            patch: parts.next()??,
        };
        parts.next().is_none().then_some(version)
    }

    /// The version of this build (from Cargo.toml).
    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Self {
            major: 0,
            minor: 0,
            patch: 0,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// This platform's package of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Package {
    name: String,
    url: String,
    checksum_url: String,
    size: u64,
}

/// The latest published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// `None` when the release has no package (with a checksum) for this platform.
    package: Option<Package>,
}

#[derive(Debug, Deserialize)]
struct ReleaseJson {
    tag_name: String,
    #[serde(default)]
    assets: Vec<AssetJson>,
}

#[derive(Debug, Deserialize)]
struct AssetJson {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

/// Reads GitHub's release description; `None` when the tag is not a version.
fn parse_release(json: ReleaseJson, target: &str) -> Option<Release> {
    let version = Version::parse(&json.tag_name)?;
    let suffix = format!("-{target}{PACKAGE_EXTENSION}");
    let package = json
        .assets
        .iter()
        .filter(|_| !target.is_empty())
        .find(|asset| asset.name.ends_with(&suffix))
        .and_then(|asset| {
            let checksum_name = format!("{}{CHECKSUM_EXTENSION}", asset.name);
            let checksum = json.assets.iter().find(|a| a.name == checksum_name)?;
            Some(Package {
                name: asset.name.clone(),
                url: asset.browser_download_url.clone(),
                checksum_url: checksum.browser_download_url.clone(),
                size: asset.size,
            })
        });
    Some(Release { version, package })
}

/// Where updating stands, as the main menu shows it.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateState {
    Checking,
    UpToDate,
    /// The check failed (offline, GitHub unreachable); nothing to offer.
    CheckFailed,
    /// A newer release exists. `installable`: this build can install it (packaged,
    /// with a package for this platform).
    Available {
        release: Release,
        installable: bool,
    },
    /// `progress` is 0–1.
    Downloading {
        version: Version,
        progress: f32,
    },
    /// The new version is in place; the game must restart into it.
    Installed {
        version: Version,
    },
    /// Installing failed (details are logged); it can be retried.
    Failed {
        release: Release,
    },
}

/// Checks for and installs updates in the background. Cloning shares the state.
#[derive(Debug, Clone)]
pub struct Updater {
    state: Arc<Mutex<UpdateState>>,
    /// The game folder when this is a packaged build.
    install_dir: Option<PathBuf>,
}

impl Updater {
    /// Starts checking for a newer release on a background thread.
    pub fn start() -> Self {
        let updater = Self {
            state: Arc::new(Mutex::new(UpdateState::Checking)),
            install_dir: install_dir(),
        };
        let state = Arc::clone(&updater.state);
        let packaged = updater.install_dir.is_some();
        let spawned = std::thread::Builder::new()
            .name("update check".to_owned())
            .spawn(move || set_state(&state, check(packaged)));
        if let Err(error) = spawned {
            tracing::warn!("could not start the update check: {error}");
            set_state(&updater.state, UpdateState::CheckFailed);
        }
        updater
    }

    pub fn state(&self) -> UpdateState {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Downloads and installs the available release on a background thread (also a
    /// retry after a failure). Does nothing in any other state.
    pub fn install(&self) {
        let release = match self.state() {
            UpdateState::Available {
                release,
                installable: true,
            }
            | UpdateState::Failed { release } => release,
            _ => return,
        };
        let (Some(dir), Some(package)) = (self.install_dir.clone(), release.package.clone()) else {
            return;
        };
        let version = release.version;
        set_state(
            &self.state,
            UpdateState::Downloading {
                version,
                progress: 0.0,
            },
        );
        let state = Arc::clone(&self.state);
        let spawned = std::thread::Builder::new()
            .name("update install".to_owned())
            .spawn(move || {
                let progress = |progress| {
                    set_state(&state, UpdateState::Downloading { version, progress });
                };
                let next = match install(&dir, &package, progress) {
                    Ok(()) => {
                        tracing::info!(%version, "update installed; restart to use it");
                        UpdateState::Installed { version }
                    }
                    Err(error) => {
                        tracing::error!("update failed: {error:#}");
                        UpdateState::Failed { release }
                    }
                };
                set_state(&state, next);
            });
        if let Err(error) = spawned {
            tracing::error!("could not start the update: {error}");
            set_state(
                &self.state,
                UpdateState::Failed {
                    release: Release {
                        version,
                        package: Some(package_placeholder()),
                    },
                },
            );
        }
    }
}

/// Keeps a failed state retryable when the install thread could not even start.
fn package_placeholder() -> Package {
    Package {
        name: String::new(),
        url: String::new(),
        checksum_url: String::new(),
        size: 0,
    }
}

fn set_state(state: &Mutex<UpdateState>, next: UpdateState) {
    *state.lock().unwrap_or_else(PoisonError::into_inner) = next;
}

/// The game folder when this is a packaged build (`assets/` beside the executable).
fn install_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    dir.join(ASSETS_DIR).is_dir().then(|| dir.to_path_buf())
}

/// Asks GitHub for the latest release and compares it with this build.
fn check(packaged: bool) -> UpdateState {
    match fetch_latest_release() {
        Ok(Some(release)) if release.version > Version::current() => {
            tracing::info!(latest = %release.version, current = %Version::current(), "update available");
            let installable = packaged && release.package.is_some();
            UpdateState::Available {
                release,
                installable,
            }
        }
        Ok(release) => {
            let latest = release.map(|r| r.version.to_string());
            tracing::info!(?latest, current = %Version::current(), "game is up to date");
            UpdateState::UpToDate
        }
        Err(error) => {
            tracing::warn!("update check failed: {error:#}");
            UpdateState::CheckFailed
        }
    }
}

/// The latest release; `None` when the repository has published none yet.
fn fetch_latest_release() -> anyhow::Result<Option<Release>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(CHECK_TIMEOUT))
        .build()
        .into();
    let response = agent
        .get(LATEST_RELEASE_URL)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call();
    let mut response = match response {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(error) => return Err(error).context("GitHub release request failed"),
    };
    let json: ReleaseJson = response
        .body_mut()
        .read_json()
        .context("unreadable release description")?;
    Ok(parse_release(json, TARGET))
}

fn download_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .build()
        .into()
}

/// Downloads, verifies, unpacks and swaps in `package` inside the game folder `dir`.
fn install(dir: &Path, package: &Package, progress: impl Fn(f32)) -> anyhow::Result<()> {
    let staging = dir.join(STAGING_DIR);
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)
        .with_context(|| format!("cannot create `{}`", staging.display()))?;

    let archive = staging.join(&package.name);
    download(&package.url, &archive, package.size, progress)?;
    let checksum = download_agent()
        .get(&package.checksum_url)
        .header("User-Agent", USER_AGENT)
        .call()
        .context("checksum download failed")?
        .body_mut()
        .read_to_string()
        .context("unreadable checksum")?;
    verify_checksum(&archive, &checksum)?;

    let unpacked = staging.join("unpacked");
    extract(&archive, &unpacked)?;
    let new_root = package_root(&unpacked)?;
    let exe = std::env::current_exe().context("cannot locate the running executable")?;
    replace_files(dir, &exe, &new_root)
}

/// Streams `url` into `dest`, reporting progress (0–1) when the size is known.
fn download(
    url: &str,
    dest: &Path,
    expected_size: u64,
    progress: impl Fn(f32),
) -> anyhow::Result<()> {
    let response = download_agent()
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .context("package download failed")?;
    let (_, body) = response.into_parts();
    let total = body
        .content_length()
        .or((expected_size > 0).then_some(expected_size));
    let mut reader = body.into_with_config().limit(MAX_PACKAGE_BYTES).reader();
    let mut file =
        File::create(dest).with_context(|| format!("cannot write `{}`", dest.display()))?;
    let mut buffer = vec![0; 64 * 1024];
    let mut received = 0u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .context("package download interrupted")?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])?;
        received += read as u64;
        if let Some(total) = total.filter(|&total| total > 0) {
            progress((received as f64 / total as f64).min(1.0) as f32);
        }
    }
    file.sync_all()?;
    Ok(())
}

/// Checks `file` against a `sha256sum` line (`<hex>  <name>`).
fn verify_checksum(file: &Path, checksum: &str) -> anyhow::Result<()> {
    let expected = checksum
        .split_whitespace()
        .next()
        .context("empty checksum file")?
        .to_ascii_lowercase();
    let actual = sha256_hex(file)?;
    if actual != expected {
        bail!("checksum mismatch: expected {expected}, got {actual}");
    }
    Ok(())
}

fn sha256_hex(path: &Path) -> anyhow::Result<String> {
    let mut file = File::open(path).with_context(|| format!("cannot read `{}`", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Unpacks a zip package (Windows). Entries escaping `dest` are rejected by the crate.
#[cfg(windows)]
fn extract(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let file = File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).context("not a zip package")?;
    zip.extract(dest).context("cannot unpack the package")?;
    Ok(())
}

/// Unpacks a tar.gz package (Linux). Entries escaping `dest` are skipped by the crate.
#[cfg(not(windows))]
fn extract(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let file = File::open(archive)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    tar.unpack(dest).context("cannot unpack the package")?;
    Ok(())
}

/// The folder holding the executable and `assets/`: the package's single top-level
/// folder, or the unpacked folder itself when the files sit at its root.
fn package_root(unpacked: &Path) -> anyhow::Result<PathBuf> {
    if unpacked.join(ASSETS_DIR).is_dir() {
        return Ok(unpacked.to_path_buf());
    }
    let mut folders = fs::read_dir(unpacked)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir());
    match (folders.next(), folders.next()) {
        (Some(root), None) if root.join(ASSETS_DIR).is_dir() => Ok(root),
        _ => bail!("unexpected package layout"),
    }
}

/// `path` with `.old` appended to its file name.
fn old_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(OLD_SUFFIX);
    path.with_file_name(name)
}

/// Swaps the game files in `dir` for those in `new_root`: the executable `exe` and the
/// `assets` folder. Everything is checked before anything moves; a failure part-way puts
/// the old files back.
fn replace_files(dir: &Path, exe: &Path, new_root: &Path) -> anyhow::Result<()> {
    let exe_name = exe.file_name().context("executable has no file name")?;
    let new_exe = new_root.join(exe_name);
    let new_assets = new_root.join(ASSETS_DIR);
    if !new_exe.is_file() || !new_assets.is_dir() {
        bail!(
            "the package lacks `{}` or `{ASSETS_DIR}/`",
            exe_name.to_string_lossy()
        );
    }
    let assets = dir.join(ASSETS_DIR);
    let old_exe = old_path(exe);
    let old_assets = old_path(&assets);
    // Leftovers of an earlier update (normally removed at startup).
    let _ = fs::remove_file(&old_exe);
    let _ = fs::remove_dir_all(&old_assets);

    fs::rename(exe, &old_exe).context("cannot move the running executable aside")?;
    if let Err(error) = fs::rename(&new_exe, exe) {
        let _ = fs::rename(&old_exe, exe);
        return Err(error).context("cannot put the new executable in place");
    }
    let restore_exe = || {
        let _ = fs::remove_file(exe);
        let _ = fs::rename(&old_exe, exe);
    };
    let had_assets = assets.exists();
    if had_assets && let Err(error) = fs::rename(&assets, &old_assets) {
        restore_exe();
        return Err(error).context("cannot move the old assets aside");
    }
    if let Err(error) = fs::rename(&new_assets, &assets) {
        if had_assets {
            let _ = fs::rename(&old_assets, &assets);
        }
        restore_exe();
        return Err(error).context("cannot put the new assets in place");
    }
    Ok(())
}

/// Removes what an earlier update left behind (the old executable and assets, and the
/// download folder). Called at startup, once the old process has exited.
pub fn clean_up_previous_update() {
    let Some(dir) = install_dir() else {
        return;
    };
    if let Ok(exe) = std::env::current_exe() {
        let _ = fs::remove_file(old_path(&exe));
    }
    let _ = fs::remove_dir_all(old_path(&dir.join(ASSETS_DIR)));
    let _ = fs::remove_dir_all(dir.join(STAGING_DIR));
}

/// Starts the (updated) executable; the caller then exits.
pub fn restart() -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("cannot locate the executable")?;
    let mut command = Command::new(&exe);
    if let Some(dir) = exe.parent() {
        command.current_dir(dir);
    }
    command.spawn().context("cannot start the updated game")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "disciples2rust-update-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn asset(name: &str) -> AssetJson {
        AssetJson {
            name: name.to_owned(),
            browser_download_url: format!("https://example.invalid/{name}"),
            size: 10,
        }
    }

    #[test]
    fn versions_parse_and_compare_numerically() {
        let v = |text| Version::parse(text).unwrap();
        assert_eq!(v("v1.2.3"), v("1.2.3"));
        assert!(v("v0.10.0") > v("v0.9.9"), "numeric, not text order");
        assert!(v("v1.0.0") > v("v0.99.99"));
        assert_eq!(v("v2.0.1-beta+7"), v("2.0.1"));
        assert_eq!(v("v1.2.3").to_string(), "v1.2.3");
        assert_eq!(Version::parse("v1.2"), None);
        assert_eq!(Version::parse("v1.2.3.4"), None);
        assert_eq!(Version::parse("latest"), None);
        assert_eq!(Version::current(), v(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn release_picks_this_platforms_package_and_checksum() {
        let target = "x86_64-pc-windows-msvc";
        let name = format!("Disciples2Rust-v0.2.0-{target}{PACKAGE_EXTENSION}");
        let json = ReleaseJson {
            tag_name: "v0.2.0".to_owned(),
            assets: vec![
                asset(&format!(
                    "Disciples2Rust-v0.2.0-other-target{PACKAGE_EXTENSION}"
                )),
                asset(&name),
                asset(&format!("{name}.sha256")),
            ],
        };
        let release = parse_release(json, target).unwrap();
        assert_eq!(release.version, Version::parse("0.2.0").unwrap());
        let package = release.package.unwrap();
        assert_eq!(package.name, name);
        assert!(package.checksum_url.ends_with(".sha256"));
    }

    #[test]
    fn release_without_a_package_or_checksum_is_not_installable() {
        let target = "x86_64-unknown-linux-gnu";
        let name = format!("Disciples2Rust-v0.2.0-{target}{PACKAGE_EXTENSION}");
        let no_checksum = ReleaseJson {
            tag_name: "v0.2.0".to_owned(),
            assets: vec![asset(&name)],
        };
        assert_eq!(parse_release(no_checksum, target).unwrap().package, None);
        let other_platform = ReleaseJson {
            tag_name: "v0.2.0".to_owned(),
            assets: Vec::new(),
        };
        assert_eq!(parse_release(other_platform, target).unwrap().package, None);
        let bad_tag = ReleaseJson {
            tag_name: "nightly".to_owned(),
            assets: Vec::new(),
        };
        assert_eq!(parse_release(bad_tag, target), None);
    }

    #[test]
    fn checksums_must_match() {
        let dir = temp_dir("checksum");
        let file = dir.join("package");
        fs::write(&file, b"abc").unwrap();
        // SHA-256 of "abc".
        let hex = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        verify_checksum(&file, &format!("{hex}  package\n")).unwrap();
        verify_checksum(&file, &hex.to_ascii_uppercase()).unwrap();
        assert!(verify_checksum(&file, &format!("{}0  package", &hex[..63])).is_err());
        assert!(verify_checksum(&file, "  \n").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_are_swapped_and_old_ones_kept() {
        let dir = temp_dir("swap");
        let exe = dir.join("game.exe");
        write(&exe, "old exe");
        write(&dir.join("assets/ui/a.txt"), "old asset");
        let new_root = dir.join(".update/unpacked/Pkg");
        write(&new_root.join("game.exe"), "new exe");
        write(&new_root.join("assets/ui/a.txt"), "new asset");

        replace_files(&dir, &exe, &new_root).unwrap();
        assert_eq!(fs::read_to_string(&exe).unwrap(), "new exe");
        assert_eq!(
            fs::read_to_string(dir.join("assets/ui/a.txt")).unwrap(),
            "new asset"
        );
        assert_eq!(
            fs::read_to_string(dir.join("game.exe.old")).unwrap(),
            "old exe"
        );
        assert_eq!(
            fs::read_to_string(dir.join("assets.old/ui/a.txt")).unwrap(),
            "old asset"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_incomplete_package_changes_nothing() {
        let dir = temp_dir("incomplete");
        let exe = dir.join("game.exe");
        write(&exe, "old exe");
        write(&dir.join("assets/a.txt"), "old asset");
        let new_root = dir.join("new");
        write(&new_root.join("game.exe"), "new exe"); // no assets/
        assert!(replace_files(&dir, &exe, &new_root).is_err());
        assert_eq!(fs::read_to_string(&exe).unwrap(), "old exe");
        assert_eq!(
            fs::read_to_string(dir.join("assets/a.txt")).unwrap(),
            "old asset"
        );
        assert!(!dir.join("game.exe.old").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn package_root_is_the_single_top_level_folder() {
        let dir = temp_dir("root");
        write(&dir.join("Pkg/assets/a.txt"), "x");
        write(&dir.join("Pkg/game.exe"), "x");
        assert_eq!(package_root(&dir).unwrap(), dir.join("Pkg"));
        let flat = temp_dir("root-flat");
        write(&flat.join("assets/a.txt"), "x");
        assert_eq!(package_root(&flat).unwrap(), flat);
        let empty = temp_dir("root-empty");
        assert!(package_root(&empty).is_err());
        for d in [dir, flat, empty] {
            let _ = fs::remove_dir_all(d);
        }
    }

    /// Builds a package the way the release workflow does and unpacks it.
    #[test]
    fn packages_unpack_with_their_top_level_folder() {
        let dir = temp_dir("extract");
        let archive = dir.join(format!("pkg{PACKAGE_EXTENSION}"));
        write_test_package(&archive);
        let unpacked = dir.join("unpacked");
        extract(&archive, &unpacked).unwrap();
        let root = package_root(&unpacked).unwrap();
        assert_eq!(fs::read_to_string(root.join("game.exe")).unwrap(), "exe");
        assert_eq!(
            fs::read_to_string(root.join("assets/a.txt")).unwrap(),
            "asset"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    fn write_test_package(path: &Path) {
        use zip::write::SimpleFileOptions;
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let options = SimpleFileOptions::default();
        zip.add_directory("Pkg/assets/", options).unwrap();
        zip.start_file("Pkg/game.exe", options).unwrap();
        zip.write_all(b"exe").unwrap();
        zip.start_file("Pkg/assets/a.txt", options).unwrap();
        zip.write_all(b"asset").unwrap();
        zip.finish().unwrap();
    }

    #[cfg(not(windows))]
    fn write_test_package(path: &Path) {
        let encoder = flate2::write::GzEncoder::new(
            File::create(path).unwrap(),
            flate2::Compression::default(),
        );
        let mut tar = tar::Builder::new(encoder);
        for (name, data) in [
            ("Pkg/game.exe", &b"exe"[..]),
            ("Pkg/assets/a.txt", &b"asset"[..]),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, name, data).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
    }
}
