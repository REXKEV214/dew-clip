use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallError {
    Missing,
    Invalid,
    Failed,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPreview {
    pub requirement_matches: bool,
    pub automatic: bool,
}

#[cfg(target_os = "macos")]
pub(super) use macos::PreparedInstall;

#[cfg(not(target_os = "macos"))]
pub(super) struct PreparedInstall;

#[cfg(not(target_os = "macos"))]
impl PreparedInstall {
    pub fn preview(&self) -> InstallPreview {
        InstallPreview {
            requirement_matches: false,
            automatic: false,
        }
    }

    pub fn launch(self) -> Result<(), InstallError> {
        Err(InstallError::Failed)
    }
}

pub(super) fn cleanup_mounts() {
    #[cfg(target_os = "macos")]
    if macos::cleanup_mounts(&macos::mount_root()).is_err() {
        crate::error!("Update mount cleanup failed");
    }
}

pub(super) fn prepare(
    cache: &super::cache::UpdateCache,
    release: &super::Release,
) -> Result<PreparedInstall, InstallError> {
    #[cfg(target_os = "macos")]
    return macos::prepare(cache, release, &macos::mount_root());
    #[cfg(not(target_os = "macos"))]
    {
        cache.verify_dmg(release).map_err(image_error)?;
        Err(InstallError::Failed)
    }
}

fn image_error(error: std::io::Error) -> InstallError {
    match error.kind() {
        std::io::ErrorKind::NotFound => InstallError::Missing,
        std::io::ErrorKind::InvalidData => InstallError::Invalid,
        _ => InstallError::Failed,
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{InstallError, InstallPreview};
    use crate::services::updater::{
        cache::{remove_file, UpdateCache},
        Release,
    };
    use std::ffi::{c_char, CString, OsStr};
    use std::fs;
    use std::io;
    use std::os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt},
        process::CommandExt,
    };
    use std::path::{Component, Path, PathBuf};
    use std::process::{Command, Stdio};

    extern "C" {
        fn dewclip_check_update_bundle(path: *const c_char, version: *const c_char) -> bool;
        fn dewclip_update_requirement(path: *const c_char, reference: *const c_char) -> i32;
    }

    fn c_path(path: &Path) -> Result<CString, InstallError> {
        CString::new(path.as_os_str().as_bytes()).map_err(|_| InstallError::Failed)
    }

    fn quiet(program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(program);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn succeeded(command: &mut Command) -> bool {
        command.status().is_ok_and(|status| status.success())
    }

    struct Mount {
        dir: PathBuf,
        attached: bool,
        handed_off: bool,
    }

    const VOLUME: &str = "volume";
    const IMAGE: &str = "image.dmg";

    pub(super) fn mount_root() -> PathBuf {
        std::env::temp_dir().join(if cfg!(debug_assertions) {
            "dewclip-update-mounts.dev"
        } else {
            "dewclip-update-mounts"
        })
    }

    fn private_dir(path: &Path) -> io::Result<fs::Metadata> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir()
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        Ok(metadata)
    }

    fn remove_dir(path: &Path) -> io::Result<()> {
        match fs::remove_dir(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }

    fn cleanup_files(work: &Path) -> io::Result<()> {
        remove_dir(&work.join(VOLUME))?;
        remove_file(&work.join(IMAGE))?;
        remove_dir(work)
    }

    fn detach_at_startup(
        volume: &Path,
        mounted_device: Option<u64>,
        running_device: u64,
        detach: &Path,
    ) -> io::Result<bool> {
        match mounted_device {
            None => Ok(true),
            Some(device) if device == running_device => Ok(false),
            Some(_) => Ok(quiet(detach)
                .args(["detach", "-quiet"])
                .arg(volume)
                .status()?
                .success()),
        }
    }

    fn cleanup_stale_mount(
        work: &Path,
        mounted_device: Option<u64>,
        running_device: u64,
        detach: &Path,
    ) -> io::Result<()> {
        if detach_at_startup(&work.join(VOLUME), mounted_device, running_device, detach)? {
            cleanup_files(work)?;
        }
        Ok(())
    }

    fn mounted_device(volume: &Path, backing_device: u64) -> io::Result<Option<u64>> {
        match fs::symlink_metadata(volume) {
            Ok(metadata) if metadata.is_dir() => {
                Ok(Some(metadata.dev()).filter(|device| *device != backing_device))
            }
            Ok(_) => Err(io::Error::from(io::ErrorKind::InvalidData)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(super) fn cleanup_mounts(root: &Path) -> io::Result<()> {
        let root_metadata = match private_dir(root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        let running_device = fs::metadata(std::env::current_exe()?)?.dev();
        let detach = Path::new("/usr/bin/hdiutil");
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if !entry
                .file_name()
                .to_str()
                .is_some_and(|name| uuid::Uuid::parse_str(name).is_ok())
            {
                continue;
            }
            let dir = entry.path();
            let metadata = fs::symlink_metadata(&dir)?;
            if !metadata.is_dir() {
                continue;
            }
            if metadata.dev() != root_metadata.dev() {
                // 较早的构建直接将工作目录用作挂载点。
                if detach_at_startup(&dir, Some(metadata.dev()), running_device, detach)? {
                    remove_dir(&dir)?;
                }
            } else {
                let device = mounted_device(&dir.join(VOLUME), metadata.dev())?;
                cleanup_stale_mount(&dir, device, running_device, detach)?;
            }
        }
        Ok(())
    }

    impl Mount {
        fn new(root: &Path) -> io::Result<Self> {
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            if let Err(error) = builder.create(root) {
                if error.kind() != io::ErrorKind::AlreadyExists {
                    return Err(error);
                }
            }
            private_dir(root)?;
            let dir = root.join(uuid::Uuid::new_v4().to_string());
            builder.create(&dir)?;
            let mount = Self {
                dir,
                attached: false,
                handed_off: false,
            };
            builder.create(mount.volume())?;
            Ok(mount)
        }

        fn volume(&self) -> PathBuf {
            self.dir.join(VOLUME)
        }

        fn snapshot(&self) -> PathBuf {
            self.dir.join(IMAGE)
        }

        fn copy_snapshot(
            &self,
            cache: &UpdateCache,
            release: &Release,
        ) -> Result<(), InstallError> {
            cache
                .copy_verified_snapshot(release, &self.snapshot())
                .map_err(super::image_error)
        }

        fn attach_command(&self) -> Command {
            let mut command = quiet("/usr/bin/hdiutil");
            command
                .args([
                    "attach",
                    "-nobrowse",
                    "-readonly",
                    "-noautoopen",
                    "-mountpoint",
                ])
                .arg(self.volume())
                .arg(self.snapshot());
            command
        }

        fn attach(&mut self) -> Result<(), InstallError> {
            if !succeeded(&mut self.attach_command()) {
                // 挂载命令报错时也可能已经挂载了一个分区。
                self.attached = fs::symlink_metadata(&self.dir)
                    .ok()
                    .and_then(|metadata| {
                        mounted_device(&self.volume(), metadata.dev())
                            .ok()
                            .flatten()
                    })
                    .is_some();
                return Err(InstallError::Failed);
            }
            self.attached = true;
            Ok(())
        }

        fn detach(&mut self) -> Result<(), InstallError> {
            if self.attached {
                if !succeeded(
                    quiet("/usr/bin/hdiutil")
                        .args(["detach", "-quiet"])
                        .arg(self.volume()),
                ) && !succeeded(
                    quiet("/usr/bin/hdiutil")
                        .args(["detach", "-force", "-quiet"])
                        .arg(self.volume()),
                ) {
                    return Err(InstallError::Failed);
                }
                self.attached = false;
            }
            Ok(())
        }

        fn reveal(&mut self, open: &mut Command) -> Result<(), InstallError> {
            if !succeeded(open.arg(self.volume())) {
                return Err(InstallError::Failed);
            }
            self.handed_off = true;
            Ok(())
        }
    }

    impl Drop for Mount {
        fn drop(&mut self) {
            if self.handed_off {
                return;
            }
            if self.detach().is_err() || cleanup_files(&self.dir).is_err() {
                crate::error!("Update image detach failed");
            }
        }
    }

    pub(in crate::services::updater) struct PreparedInstall {
        mount: Mount,
        source: PathBuf,
        target: Option<PathBuf>,
        matches: bool,
    }

    impl PreparedInstall {
        pub fn preview(&self) -> InstallPreview {
            InstallPreview {
                requirement_matches: self.matches,
                automatic: self.target.is_some(),
            }
        }

        pub fn launch(self) -> Result<(), InstallError> {
            self.launch_with_open(Path::new("/usr/bin/open"))
        }

        fn launch_with_open(mut self, open: &Path) -> Result<(), InstallError> {
            if let Some(target) = &self.target {
                let parent = target.parent().ok_or(InstallError::Failed)?;
                let id = uuid::Uuid::new_v4();
                let staging = parent.join(format!(".DewClip.app.stage.{id}"));
                let backup = parent.join(format!(".DewClip.app.backup.{id}"));
                let mut helper = replacement_command(
                    std::process::id(),
                    [
                        &self.mount.volume(),
                        &self.source,
                        target,
                        &staging,
                        &backup,
                        open,
                    ],
                );
                unsafe {
                    helper.pre_exec(|| {
                        if libc::setsid() == -1 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
                helper.spawn().map_err(|_| InstallError::Failed)?;
                self.mount.handed_off = true;
            } else {
                self.mount.reveal(&mut quiet(open))?;
            }
            Ok(())
        }
    }

    fn replacement_command(pid: u32, paths: [&Path; 6]) -> Command {
        let mut command = quiet("/bin/sh");
        command
            .args(["-c", include_str!("install.sh"), "dewclip-install"])
            .arg(pid.to_string())
            .args(paths);
        command
    }

    fn writable(path: &Path) -> bool {
        fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
            && c_path(path).is_ok_and(|path| unsafe {
                libc::access(path.as_ptr(), libc::W_OK | libc::X_OK) == 0
            })
    }

    fn replacement_target(target: &Path, portable: bool) -> bool {
        !portable
            && !target.starts_with("/Volumes")
            && !target
                .components()
                .any(|component| component == Component::Normal("AppTranslocation".as_ref()))
            && target.parent().is_some_and(writable)
            && writable(target)
    }

    fn running_bundle() -> Option<PathBuf> {
        let executable = std::env::current_exe().ok()?.canonicalize().ok()?;
        let macos = executable.parent()?;
        let contents = macos.parent()?;
        let bundle = contents.parent()?;
        (macos.ends_with("MacOS")
            && contents.ends_with("Contents")
            && bundle.extension().is_some_and(|ext| ext == "app"))
        .then(|| bundle.to_owned())
    }

    pub(super) fn prepare(
        cache: &UpdateCache,
        release: &Release,
        mount_root: &Path,
    ) -> Result<PreparedInstall, InstallError> {
        let mut mount = Mount::new(mount_root).map_err(|_| InstallError::Failed)?;
        mount.copy_snapshot(cache, release)?;
        mount.attach()?;
        let entries = fs::read_dir(mount.volume()).map_err(|_| InstallError::Failed)?;
        let bundles = entries
            .filter_map(|entry| match entry {
                Ok(entry) if entry.path().extension().is_some_and(|ext| ext == "app") => {
                    Some(Ok(entry.path()))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| InstallError::Failed)?;
        let [source] = bundles.as_slice() else {
            return Err(InstallError::Failed);
        };
        if !source.ends_with("DewClip.app")
            || !fs::symlink_metadata(source).is_ok_and(|metadata| metadata.is_dir())
            || !fs::symlink_metadata(source.join("Contents"))
                .is_ok_and(|metadata| metadata.is_dir())
            || !fs::symlink_metadata(source.join("Contents/Info.plist"))
                .is_ok_and(|metadata| metadata.is_file())
        {
            return Err(InstallError::Failed);
        }
        let path = c_path(source)?;
        let version =
            CString::new(release.version.to_string()).map_err(|_| InstallError::Failed)?;
        if !unsafe { dewclip_check_update_bundle(path.as_ptr(), version.as_ptr()) }
            || !succeeded(
                quiet("/usr/bin/codesign")
                    .args(["--verify", "--deep", "--strict"])
                    .arg(source),
            )
        {
            return Err(InstallError::Failed);
        }
        let matches = match unsafe { dewclip_update_requirement(path.as_ptr(), std::ptr::null()) } {
            1 => true,
            0 => false,
            _ => return Err(InstallError::Failed),
        };
        let portable =
            cfg!(feature = "portable") || crate::app::setup::portable_data_dir().is_some();
        let target =
            running_bundle().filter(|target| matches && replacement_target(target, portable));
        Ok(PreparedInstall {
            mount,
            source: source.clone(),
            target,
            matches,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::services::updater::{cache::TestDirectory, http, ReleaseAsset};
        use semver::Version;
        use sha2::{Digest, Sha256};
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};

        fn signed_bundle(parent: &Path, version: &str) -> PathBuf {
            let bundle = parent.join("DewClip.app");
            let contents = bundle.join("Contents");
            fs::create_dir_all(contents.join("MacOS")).unwrap();
            fs::copy("/usr/bin/true", contents.join("MacOS/DewClip")).unwrap();
            fs::write(
                contents.join("Info.plist"),
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>io.github.rexkev214.dewclip</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundleExecutable</key><string>DewClip</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>"#
                ),
            )
            .unwrap();
            let sign = Command::new("/usr/bin/codesign")
                .args(["--force", "--sign", "-"])
                .arg(&bundle)
                .output()
                .unwrap();
            assert!(
                sign.status.success(),
                "{}",
                String::from_utf8_lossy(&sign.stderr)
            );
            assert!(succeeded(
                quiet("/usr/bin/codesign")
                    .args(["--verify", "--deep", "--strict"])
                    .arg(&bundle)
            ));
            bundle
        }

        fn cache_signed_test_dmg(
            dirs: &TestDirectory,
            bundle_version: &str,
            test: &str,
        ) -> Option<(UpdateCache, Release)> {
            let source = dirs.0.join("source");
            signed_bundle(&source, bundle_version);
            let image = dirs.0.join("fixture.dmg");
            let output = Command::new("/usr/bin/hdiutil")
                .arg("create")
                .arg("-srcfolder")
                .arg(&source)
                .args(["-format", "UDZO", "-fs", "HFS+"])
                .arg(&image)
                .stdin(Stdio::null())
                .output()
                .unwrap();
            if !output.status.success() {
                let error = String::from_utf8_lossy(&output.stderr);
                if ["Device not configured", "Operation not permitted"]
                    .iter()
                    .any(|reason| error.contains(reason))
                {
                    writeln!(io::stderr().lock(), "Skipping {test}: {}", error.trim()).unwrap();
                    return None;
                }
                panic!("hdiutil create failed for {test}: {}", error.trim());
            }

            let bytes = fs::read(&image).unwrap();
            let version = Version::new(1, 2, 0);
            let name = http::dmg_name(&version);
            let release = Release {
                version,
                release_url: format!("{}tag/v1.2.0", http::RELEASES),
                notes_preview: String::new(),
                asset: Some(ReleaseAsset {
                    size: bytes.len() as u64,
                    sha256: format!("{:x}", Sha256::digest(&bytes)),
                    download_url: format!("{}v1.2.0/{name}", http::DOWNLOADS),
                }),
            };
            let cache = dirs.updates();
            cache.ensure_dir().unwrap();
            let mut part = cache.temporary(&name).unwrap();
            part.file.write_all(&bytes).unwrap();
            part.file.sync_all().unwrap();
            cache.commit(release, part).unwrap();
            let ready = cache.load(&Version::new(1, 1, 0), None).unwrap().unwrap();
            Some((cache, ready.release))
        }

        fn image_is_attached(path: &Path) -> bool {
            let output = Command::new("/usr/bin/hdiutil")
                .arg("info")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).contains(path.to_str().unwrap())
        }

        #[test]
        fn real_dmg_preparation_accepts_ad_hoc_bundle_and_detaches_on_drop() {
            let dirs = TestDirectory::new();
            let Some((cache, release)) = cache_signed_test_dmg(
                &dirs,
                "1.2.0",
                "real_dmg_preparation_accepts_ad_hoc_bundle_and_detaches_on_drop",
            ) else {
                return;
            };
            let mounts = dirs.0.join("mounts");
            assert!(running_bundle().is_none());
            let prepared = prepare(&cache, &release, &mounts).expect("prepare the signed image");
            let volume = prepared.mount.volume();
            let snapshot = prepared.mount.snapshot();
            assert_eq!(prepared.source, volume.join("DewClip.app"));
            assert!(prepared.source.is_dir());
            assert!(snapshot.is_file());
            assert!(image_is_attached(&snapshot));
            let preview = prepared.preview();
            assert!(!preview.requirement_matches);
            assert!(!preview.automatic);

            drop(prepared);
            assert!(!volume.exists());
            assert!(!snapshot.exists());
            assert!(fs::read_dir(&mounts).unwrap().next().is_none());
            assert!(!image_is_attached(&mounts));
        }

        #[test]
        fn real_dmg_preparation_rejects_wrong_bundle_version_and_cleans_up() {
            let dirs = TestDirectory::new();
            let Some((cache, release)) = cache_signed_test_dmg(
                &dirs,
                "1.3.0",
                "real_dmg_preparation_rejects_wrong_bundle_version_and_cleans_up",
            ) else {
                return;
            };
            let mounts = dirs.0.join("mounts");
            assert!(matches!(
                prepare(&cache, &release, &mounts),
                Err(InstallError::Failed)
            ));
            assert!(fs::read_dir(&mounts).unwrap().next().is_none());
            assert!(!image_is_attached(&mounts));
        }

        #[test]
        fn ad_hoc_bundles_satisfy_only_their_own_designated_requirements() {
            let dirs = TestDirectory::new();
            let old = signed_bundle(&dirs.0.join("old"), "1.2.0");
            let new = signed_bundle(&dirs.0.join("new"), "1.3.0");
            let old = c_path(&old).unwrap();
            let new = c_path(&new).unwrap();
            unsafe {
                assert_eq!(dewclip_update_requirement(old.as_ptr(), old.as_ptr()), 1);
                assert_eq!(dewclip_update_requirement(new.as_ptr(), new.as_ptr()), 1);
                assert_eq!(dewclip_update_requirement(new.as_ptr(), old.as_ptr()), 0);
                assert!(dewclip_check_update_bundle(new.as_ptr(), c"1.3.0".as_ptr()));
                assert!(!dewclip_check_update_bundle(
                    new.as_ptr(),
                    c"1.2.0".as_ptr()
                ));
                assert_eq!(
                    dewclip_update_requirement(new.as_ptr(), std::ptr::null()),
                    0
                );
            }
        }

        #[test]
        fn unwritable_portable_and_translocated_bundles_use_manual_install() {
            let dirs = TestDirectory::new();
            let parent = dirs.0.join("install");
            let bundle = parent.join("DewClip.app");
            fs::create_dir_all(&bundle).unwrap();
            assert!(replacement_target(&bundle, false));
            assert!(!replacement_target(&bundle, true));
            fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();
            let writable = replacement_target(&bundle, false);
            fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(!writable);
            assert!(!replacement_target(
                Path::new("/Volumes/DewClip/DewClip.app"),
                false
            ));
            let translocated = dirs.0.join("AppTranslocation/volume/DewClip.app");
            fs::create_dir_all(&translocated).unwrap();
            assert!(!replacement_target(&translocated, false));
        }

        #[test]
        fn helper_preserves_modes_and_keeps_the_backup_until_open_succeeds() {
            for (copy_succeeds, open_succeeds, opened) in [
                (true, true, "new\n"),
                (false, true, "old\n"),
                (true, false, "new\nold\n"),
            ] {
                let dirs = TestDirectory::new();
                let mut handoff = Mount::new(&dirs.0.join("mounts")).unwrap();
                let mount = handoff.volume();
                let snapshot = handoff.snapshot();
                fs::write(&snapshot, b"image").unwrap();
                handoff.handed_off = true;
                let source = dirs.0.join("Installer/DewClip.app");
                let target = dirs.0.join("Application Folder/DewClip.app");
                let stage = target.with_file_name(".DewClip.stage");
                let backup = target.with_file_name(".DewClip.backup");
                let launch_log = target.with_file_name("opened");
                let open = target.with_file_name("open-test");
                fs::create_dir_all(&target).unwrap();
                fs::write(target.join("old"), b"old").unwrap();
                fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
                fs::write(
                    &open,
                    r#"#!/bin/sh
[ "$1" = "-n" ] || exit 1
app=$2
parent=$(/usr/bin/dirname "$app")
if [ -f "$app/new" ]; then
    echo new >> "$parent/opened"
    [ -f "$parent/.DewClip.backup/old" ] && [ ! -e "$parent/reject-new" ]
else
    [ -f "$app/old" ] && echo old >> "$parent/opened"
fi
"#,
                )
                .unwrap();
                fs::set_permissions(&open, fs::Permissions::from_mode(0o700)).unwrap();
                if !open_succeeds {
                    fs::write(target.with_file_name("reject-new"), []).unwrap();
                }
                let bundle_mode =
                    |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
                assert_eq!(bundle_mode(&target), 0o755);
                if copy_succeeds {
                    fs::create_dir_all(&source).unwrap();
                    fs::write(source.join("new"), b"new").unwrap();
                    fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
                    assert_eq!(bundle_mode(&source), 0o755);
                }
                let mut process = quiet("/bin/sleep").arg("60").spawn().unwrap();
                let mut helper = replacement_command(
                    process.id(),
                    [&mount, &source, &target, &stage, &backup, &open],
                )
                .spawn()
                .unwrap();
                std::thread::sleep(Duration::from_millis(150));
                let untouched = target.join("old").is_file() && !stage.exists() && !backup.exists();
                process.kill().unwrap();
                process.wait().unwrap();
                let success = copy_succeeds && open_succeeds;
                assert_eq!(
                    helper.wait().unwrap().code(),
                    Some(if success { 0 } else { 1 })
                );
                assert_eq!(fs::read_to_string(launch_log).unwrap(), opened);
                assert!(untouched);
                assert_eq!(target.join("new").is_file(), success);
                assert_eq!(target.join("old").is_file(), !success);
                assert_eq!(bundle_mode(&target), 0o755);
                assert!(!stage.exists());
                assert!(!backup.exists());
                assert!(!mount.exists());
                assert!(!snapshot.exists());
                assert!(!handoff.dir.exists());
            }
        }

        #[test]
        fn finder_reveal_hands_off_the_verified_mount_for_startup_cleanup() {
            let dirs = TestDirectory::new();
            let root = dirs.0.join("mounts");
            let mount = Mount::new(&root).unwrap();
            let path = mount.volume();
            let snapshot = mount.snapshot();
            fs::write(&snapshot, b"image").unwrap();
            let unrelated = root.join("other");
            fs::create_dir(&unrelated).unwrap();
            let alias = root.join(uuid::Uuid::new_v4().to_string());
            std::os::unix::fs::symlink(&unrelated, &alias).unwrap();
            let install = PreparedInstall {
                source: path.join("DewClip.app"),
                mount,
                target: None,
                matches: false,
            };
            let open = dirs.0.join("open-test");
            fs::write(
                &open,
                r#"#!/bin/sh
[ "$#" = 1 ] && [ -d "$1" ] && printf '%s' "$1" > "$0.received"
"#,
            )
            .unwrap();
            fs::set_permissions(&open, fs::Permissions::from_mode(0o700)).unwrap();
            install.launch_with_open(&open).unwrap();
            assert_eq!(
                fs::read_to_string(open.with_extension("received")).unwrap(),
                path.to_str().unwrap()
            );
            assert!(path.is_dir());
            assert!(snapshot.is_file());

            cleanup_mounts(&root).unwrap();
            assert!(!path.parent().unwrap().exists());
            assert!(unrelated.is_dir());
            assert!(fs::symlink_metadata(alias).unwrap().is_symlink());
        }

        #[test]
        fn attach_uses_the_verified_private_snapshot_after_the_cache_file_is_replaced() {
            let dirs = TestDirectory::new();
            let cache = dirs.updates();
            cache.ensure_dir().unwrap();
            let original = b"verified release image";
            let release = Release {
                version: Version::new(1, 2, 0),
                release_url: http::RELEASES.into(),
                notes_preview: String::new(),
                asset: Some(ReleaseAsset {
                    size: original.len() as u64,
                    sha256: format!("{:x}", Sha256::digest(original)),
                    download_url: format!("{}v1.2.0/DewClip_1.2.0.dmg", http::DOWNLOADS),
                }),
            };
            let cached_image = cache.path(&release.version);
            fs::write(&cached_image, original).unwrap();
            let attribute = "com.dewclip.snapshot-test";
            assert!(succeeded(
                quiet("/usr/bin/xattr")
                    .args(["-w", attribute, "copied"])
                    .arg(&cached_image)
            ));
            let mounts = dirs.0.join("mounts");
            let mount = Mount::new(&mounts).unwrap();
            mount.copy_snapshot(&cache, &release).unwrap();

            let replacement = cache.root.join("replacement");
            fs::write(&replacement, b"changed release image!").unwrap();
            fs::rename(&replacement, &cached_image).unwrap();
            let command = mount.attach_command();
            let image_to_attach = Path::new(command.get_args().last().unwrap());
            assert_eq!(image_to_attach, mount.snapshot());
            assert_ne!(image_to_attach, cached_image);
            assert_eq!(fs::read(image_to_attach).unwrap(), original);
            let copied = Command::new("/usr/bin/xattr")
                .args(["-p", attribute])
                .arg(image_to_attach)
                .output()
                .unwrap();
            assert!(copied.status.success());
            assert_eq!(copied.stdout, b"copied\n");
            assert_eq!(fs::metadata(&mount.dir).unwrap().mode() & 0o777, 0o700);
            assert_eq!(fs::metadata(image_to_attach).unwrap().mode() & 0o777, 0o600);

            let reject = Mount::new(&mounts).unwrap();
            assert!(matches!(
                reject.copy_snapshot(&cache, &release),
                Err(InstallError::Invalid)
            ));
            fs::remove_file(&cached_image).unwrap();
            std::os::unix::fs::symlink(image_to_attach, &cached_image).unwrap();
            let symlink = Mount::new(&mounts).unwrap();
            assert!(matches!(
                symlink.copy_snapshot(&cache, &release),
                Err(InstallError::Invalid)
            ));
            assert!(!symlink.snapshot().exists());

            let work = mount.dir.clone();
            drop(mount);
            assert!(!work.exists());
        }

        #[test]
        fn startup_leaves_busy_volumes_and_the_running_apps_volume_attached() {
            let dirs = TestDirectory::new();
            let mut mount = Mount::new(&dirs.0.join("mounts")).unwrap();
            fs::write(mount.snapshot(), b"image").unwrap();
            mount.handed_off = true;
            let detach = dirs.0.join("detach-test");
            let marker = dirs.0.join("detach-test.busy");
            let calls = dirs.0.join("detach-test.calls");
            fs::write(
                &detach,
                r#"#!/bin/sh
printf '%s\n' "$*" >> "$0.calls"
[ "$#" = 3 ] && [ "$1" = detach ] && [ "$2" = "-quiet" ] && [ ! -e "$0.busy" ]
"#,
            )
            .unwrap();
            fs::set_permissions(&detach, fs::Permissions::from_mode(0o700)).unwrap();
            fs::write(&marker, []).unwrap();

            let running = fs::metadata(std::env::current_exe().unwrap())
                .unwrap()
                .dev();
            let other = fs::metadata("/dev").unwrap().dev();
            assert_ne!(running, other);
            cleanup_stale_mount(&mount.dir, Some(running), running, &detach).unwrap();
            assert!(!calls.exists());
            assert!(mount.volume().is_dir());

            cleanup_stale_mount(&mount.dir, Some(other), running, &detach).unwrap();
            let normal_detach = format!("detach -quiet {}\n", mount.volume().display());
            assert_eq!(fs::read_to_string(&calls).unwrap(), normal_detach);
            assert!(mount.volume().is_dir());
            assert!(mount.snapshot().is_file());

            fs::remove_file(marker).unwrap();
            cleanup_stale_mount(&mount.dir, Some(other), running, &detach).unwrap();
            assert_eq!(fs::read_to_string(&calls).unwrap(), normal_detach.repeat(2));
            assert!(!mount.dir.exists());
        }

        #[test]
        fn helper_times_out_without_touching_the_running_bundle() {
            let dirs = TestDirectory::new();
            let mut handoff = Mount::new(&dirs.0.join("mounts")).unwrap();
            let mount = handoff.volume();
            let snapshot = handoff.snapshot();
            fs::write(&snapshot, b"image").unwrap();
            handoff.handed_off = true;
            let source = dirs.0.join("New.app");
            let target = dirs.0.join("DewClip.app");
            let stage = dirs.0.join(".DewClip.stage");
            let backup = dirs.0.join(".DewClip.backup");
            for path in [&source, &target] {
                fs::create_dir(path).unwrap();
            }
            fs::write(target.join("old"), b"old").unwrap();
            fs::write(source.join("new"), b"new").unwrap();
            let mut process = quiet("/bin/sleep").arg("60").spawn().unwrap();
            let started = Instant::now();
            let status = replacement_command(
                process.id(),
                [
                    &mount,
                    &source,
                    &target,
                    &stage,
                    &backup,
                    Path::new("/usr/bin/false"),
                ],
            )
            .status();
            process.kill().unwrap();
            process.wait().unwrap();
            assert_eq!(status.unwrap().code(), Some(1));
            assert!(started.elapsed() < Duration::from_secs(18));
            assert!(target.join("old").exists());
            assert!(!target.join("new").exists());
            assert!(!stage.exists());
            assert!(!backup.exists());
            assert!(!mount.exists());
            assert!(!snapshot.exists());
            assert!(!handoff.dir.exists());
        }
    }
}
