use super::{http, Release, ReleaseAsset};
use semver::Version;
use sha2::{Digest, Sha256};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

const MANIFEST: &str = "ready.json";
pub(super) const RELEASE_LIMIT: u64 = 4096;

pub(super) struct Part {
    pub file: File,
    path: PathBuf,
}

impl Part {
    fn persist(&self, target: &Path) -> io::Result<()> {
        fs::rename(&self.path, target)
    }
}

impl Drop for Part {
    fn drop(&mut self) {
        if remove_file(&self.path).is_err() {
            crate::error!("Update partial file cleanup failed");
        }
    }
}

#[derive(Clone)]
pub(super) struct Ready {
    pub release: Release,
    fingerprint: Fingerprint,
}

#[derive(PartialEq, Eq, Clone)]
struct Fingerprint {
    size: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

impl Fingerprint {
    fn from(metadata: Metadata) -> io::Result<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        if !metadata.is_file() {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        Ok(Self {
            size: metadata.len(),
            modified: metadata.modified()?,
            #[cfg(unix)]
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        })
    }
}

#[derive(Clone)]
pub(crate) struct UpdateCache {
    pub(super) root: PathBuf,
    pub(super) usable: bool,
}

fn overlaps_data_dir(cache: &Path, data_dir: &Path) -> io::Result<bool> {
    let data_dir = data_dir.canonicalize()?;
    for ancestor in cache.ancestors() {
        match ancestor.canonicalize() {
            Ok(existing) => {
                return Ok(existing.starts_with(&data_dir)
                    || (ancestor == cache && data_dir.starts_with(&existing)));
            }
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    && fs::symlink_metadata(ancestor)
                        .is_err_and(|error| error.kind() == io::ErrorKind::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::from(io::ErrorKind::InvalidInput))
}

fn require_dir(path: &Path, create: bool) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(io::Error::from(io::ErrorKind::InvalidData)),
        Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
            if let Err(error) = fs::create_dir(path) {
                if error.kind() != io::ErrorKind::AlreadyExists {
                    return Err(error);
                }
            }
            require_dir(path, false)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn read_regular(path: &Path) -> io::Result<File> {
    Fingerprint::from(fs::symlink_metadata(path)?)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    Fingerprint::from(file.metadata()?)?;
    Ok(file)
}

pub(super) fn remove_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn verify_file(path: &Path, asset: &ReleaseAsset) -> io::Result<Fingerprint> {
    let file = read_regular(path)?;
    let fingerprint = Fingerprint::from(file.metadata()?)?;
    if fingerprint.size != asset.size {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    let mut digest = Sha256::new();
    let bytes = io::copy(&mut file.take(asset.size + 1), &mut digest)?;
    if bytes != asset.size
        || format!("{:x}", digest.finalize()) != asset.sha256
        || Fingerprint::from(fs::symlink_metadata(path)?)? != fingerprint
    {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    Ok(fingerprint)
}

fn remove_owned_file(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() || metadata.is_symlink() => remove_file(path),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

impl UpdateCache {
    pub fn new(root: PathBuf) -> Self {
        Self { root, usable: true }
    }

    pub fn exclude_data_dir(&mut self, data_dir: &Path) {
        self.usable &= overlaps_data_dir(&self.root, data_dir).is_ok_and(|overlaps| !overlaps);
    }

    pub(super) fn path(&self, version: &Version) -> PathBuf {
        self.root.join(http::dmg_name(version))
    }

    fn directory(&self, create: bool) -> io::Result<bool> {
        if !self.usable {
            return Ok(false);
        }
        let parent = self.root.parent().ok_or(io::ErrorKind::InvalidInput)?;
        for ancestor in parent
            .ancestors()
            .skip(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            if !require_dir(ancestor, false)? {
                return Ok(false);
            }
        }
        Ok(require_dir(parent, create)? && require_dir(&self.root, create)?)
    }

    pub(super) fn temporary(&self, name: &str) -> io::Result<Part> {
        if !self.directory(false)? {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        let path = self
            .root
            .join(format!(".{name}.{}.part", uuid::Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
        Ok(Part { file, path })
    }

    pub(super) fn ensure_dir(&self) -> io::Result<()> {
        if !self.directory(true)? {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        Ok(())
    }

    pub(super) fn load(
        &self,
        current: &Version,
        skipped: Option<&Version>,
    ) -> io::Result<Option<Ready>> {
        if !self.directory(false)? {
            return Ok(None);
        }
        let manifest = self.root.join(MANIFEST);
        let ready = read_regular(&manifest)
            .ok()
            .and_then(|file| {
                let mut json = Vec::new();
                file.take(RELEASE_LIMIT + 1).read_to_end(&mut json).ok()?;
                (json.len() as u64 <= RELEASE_LIMIT).then_some(json)
            })
            .and_then(|json| serde_json::from_slice::<Release>(&json).ok())
            .and_then(|mut release| {
                release.validate_cached();
                let asset = release.asset.as_ref()?;
                if !release.version.pre.is_empty()
                    || release.version.to_string().len() > 128
                    || !http::newer_than(&release.version, current)
                    || skipped.is_some_and(|version| !http::newer_than(&release.version, version))
                {
                    return None;
                }
                let fingerprint = self.verify(&release.version, asset).ok()?;
                Some(Ready {
                    release,
                    fingerprint,
                })
            });
        if ready.is_none() {
            remove_owned_file(&manifest)?;
        }
        self.clean_downloads(ready.as_ref().map(|ready| &ready.release.version), true)?;
        Ok(ready)
    }

    fn clean_downloads(&self, keep: Option<&Version>, parts: bool) -> io::Result<()> {
        let keep = keep.map(http::dmg_name);
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let dmg = name
                .strip_prefix("DewClip_")
                .and_then(|name| name.strip_suffix(".dmg"))
                .is_some_and(|version| Version::parse(version).is_ok());
            if (dmg && keep.as_deref() != Some(name))
                || (parts && name.starts_with(".DewClip_") && name.ends_with(".part"))
            {
                remove_owned_file(&entry.path())?;
            }
        }
        Ok(())
    }

    pub(super) fn clear(&self) -> io::Result<()> {
        if !self.directory(false)? {
            return Ok(());
        }
        self.clean_downloads(None, true)?;
        remove_owned_file(&self.root.join(MANIFEST))
    }

    pub(super) fn unchanged(&self, ready: &Ready) -> bool {
        self.directory(false).unwrap_or(false)
            && fs::symlink_metadata(self.path(&ready.release.version))
                .and_then(Fingerprint::from)
                .is_ok_and(|fingerprint| fingerprint == ready.fingerprint)
    }

    pub(super) fn forget(&self, version: &Version) -> io::Result<()> {
        if self.directory(false)? {
            remove_owned_file(&self.root.join(MANIFEST))?;
            remove_owned_file(&self.path(version))?;
        }
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn verify_dmg(&self, release: &Release) -> io::Result<()> {
        let asset = release.asset.as_ref().ok_or(io::ErrorKind::InvalidData)?;
        self.verify(&release.version, asset).map(|_| ())
    }

    #[cfg(target_os = "macos")]
    pub(super) fn copy_verified_snapshot(
        &self,
        release: &Release,
        snapshot: &Path,
    ) -> io::Result<()> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;

        let asset = release.asset.as_ref().ok_or(io::ErrorKind::InvalidData)?;
        if !self.directory(false)? {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        let source = read_regular(&self.path(&release.version))?;
        if source.metadata()?.len() != asset.size {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(snapshot)?;
        io::copy(&mut (&source).take(asset.size + 1), &mut output)?;
        // 复制扩展属性，包括系统附加的隔离属性。
        if unsafe {
            libc::fcopyfile(
                source.as_raw_fd(),
                output.as_raw_fd(),
                std::ptr::null_mut(),
                libc::COPYFILE_XATTR,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        drop(output);
        verify_file(snapshot, asset).map(|_| ())
    }

    fn verify(&self, version: &Version, asset: &ReleaseAsset) -> io::Result<Fingerprint> {
        if !self.directory(false)? {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        verify_file(&self.path(version), asset)
    }

    pub(super) fn commit(&self, release: Release, part: Part) -> io::Result<Ready> {
        let manifest = serde_json::to_vec(&release)?;
        if manifest.len() as u64 > RELEASE_LIMIT || !self.directory(false)? {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        let mut staged_manifest = self.temporary(&http::dmg_name(&release.version))?;
        staged_manifest.file.write_all(&manifest)?;
        staged_manifest.file.sync_all()?;

        let path = self.path(&release.version);
        part.persist(&path)?;
        let fingerprint =
            part.file
                .metadata()
                .and_then(Fingerprint::from)
                .and_then(|fingerprint| {
                    staged_manifest.persist(&self.root.join(MANIFEST))?;
                    Ok(fingerprint)
                });
        let fingerprint = match fingerprint {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                remove_file(&path)?;
                return Err(error);
            }
        };
        if self.clean_downloads(Some(&release.version), false).is_err() {
            crate::error!("Update cache cleanup failed");
        }
        Ok(Ready {
            release,
            fingerprint,
        })
    }
}

#[cfg(test)]
pub(super) struct TestDirectory(pub PathBuf);

#[cfg(test)]
impl TestDirectory {
    pub fn new() -> Self {
        let path = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("dewclip-updater-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    pub fn updates(&self) -> UpdateCache {
        UpdateCache::new(self.0.join("updates"))
    }
}

#[cfg(test)]
impl Drop for TestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
