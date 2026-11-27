use crate::infrastructure::app_path::canonical_app_path;
use base64::Engine;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[cfg(target_os = "macos")]
use objc2::rc::autoreleasepool;
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::AnyObject;
#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSBitmapImageRepPropertyKey, NSImage, NSWorkspace,
};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSArray, NSDictionary, NSString, NSURL};

#[cfg(target_os = "macos")]
#[link(name = "UniformTypeIdentifiers", kind = "framework")]
extern "C" {}

static EXECUTABLE_ICON_CACHE: OnceLock<Mutex<IconCache>> = OnceLock::new();
static FILE_ICON_CACHE: OnceLock<Mutex<IconCache>> = OnceLock::new();

/// Icons kept per cache; file icons are keyed by path, so without a limit the
/// cache grew with every file ever copied.
const ICON_CACHE_LIMIT: usize = 256;
/// Largest icon side sent to the page: cards show icons at 16-30 pt (2x).
const ICON_PIXELS: u32 = 64;

/// Icons by key, the oldest dropped first past the limit.
#[derive(Default)]
struct IconCache {
    icons: HashMap<String, Option<String>>,
    order: std::collections::VecDeque<String>,
}

impl IconCache {
    fn get(&self, key: &str) -> Option<Option<String>> {
        self.icons.get(key).cloned()
    }

    fn insert(&mut self, key: String, icon: Option<String>) {
        if self.icons.insert(key.clone(), icon).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > ICON_CACHE_LIMIT {
            if let Some(oldest) = self.order.pop_front() {
                self.icons.remove(&oldest);
            }
        }
    }
}

/// The icon at display size. NSWorkspace icons carry up to 1024 px
/// representations, which made each one tens to hundreds of KB of base64.
fn shrink_icon_png(png: Vec<u8>) -> Vec<u8> {
    let Ok(image) = image::load_from_memory_with_format(&png, image::ImageFormat::Png) else {
        return png;
    };
    if image.width() <= ICON_PIXELS && image.height() <= ICON_PIXELS {
        return png;
    }
    let small = image.resize(ICON_PIXELS, ICON_PIXELS, image::imageops::FilterType::Lanczos3);
    let mut out = Vec::new();
    match small.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png) {
        Ok(()) => out,
        Err(_) => png,
    }
}

pub async fn launch_uwp_with_file(
    _package: &str,
    _file: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // macOS equivalent: open -a or similar
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum AppAssociation {
    WebLink,
    Extension(String),
}

fn app_association(content_type: &str) -> AppAssociation {
    let normalized = content_type.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "url" | "link" => AppAssociation::WebLink,
        "text" | "code" => AppAssociation::Extension("txt".into()),
        "rich_text" => AppAssociation::Extension("html".into()),
        "image" => AppAssociation::Extension("png".into()),
        "video" => AppAssociation::Extension("mp4".into()),
        _ => AppAssociation::Extension(normalized.trim_start_matches('.').into()),
    }
}

fn app_bundle(path: &Path) -> Option<(PathBuf, String)> {
    let path = canonical_app_path(path)?;
    let name = path.file_stem()?.to_string_lossy().into_owned();
    Some((path, name))
}

fn ordered_app_bundles(paths: impl IntoIterator<Item = PathBuf>, default: Option<&Path>) -> Vec<Value> {
    let default = default.and_then(app_bundle);
    let mut seen = HashSet::new();
    let mut apps: Vec<_> = default
        .iter()
        .cloned()
        .chain(paths.into_iter().filter_map(|path| app_bundle(&path)))
        .filter(|(path, _)| seen.insert(path.clone()))
        .collect();
    apps.sort_by_cached_key(|(path, name)| (
        default.as_ref().is_none_or(|(default_path, _)| default_path != path),
        name.to_lowercase(),
        path.clone(),
    ));
    apps.into_iter()
        .map(|(path, name)| json!({ "name": name, "path": path.to_string_lossy() }))
        .collect()
}

pub fn scan_installed_apps() -> Vec<Value> {
    let mut roots: Vec<PathBuf> = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ];

    if let Ok(home) = std::env::var("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }

    let paths = roots.into_iter()
        .filter_map(|root| std::fs::read_dir(root).ok())
        .flat_map(|entries| entries.filter_map(|entry| entry.ok().map(|entry| entry.path())));
    ordered_app_bundles(paths, None)
}

fn lookup_apps(
    content_type: &str,
    include_associated: bool,
) -> (Option<Retained<NSURL>>, Option<Retained<NSArray<NSURL>>>) {
    let workspace = NSWorkspace::sharedWorkspace();
    match app_association(content_type) {
        AppAssociation::WebLink => {
            let url = NSURL::URLWithString(&NSString::from_str("https://example.com"))
                .expect("valid HTTPS URL");
            (
                workspace.URLForApplicationToOpenURL(&url),
                include_associated.then(|| workspace.URLsForApplicationsToOpenURL(&url)),
            )
        }
        AppAssociation::Extension(ext) if !ext.is_empty() => unsafe {
            // The offline objc2 crates lack a binding for UTType.
            let ty: Option<Retained<AnyObject>> = objc2::msg_send![
                objc2::class!(UTType), typeWithFilenameExtension: &*NSString::from_str(&ext)
            ];
            let Some(ty) = ty else {
                return (None, None);
            };
            let default = objc2::msg_send![&*workspace, URLForApplicationToOpenContentType: &*ty];
            let associated = include_associated.then(|| {
                objc2::msg_send![&*workspace, URLsForApplicationsToOpenContentType: &*ty]
            });
            (default, associated)
        },
        AppAssociation::Extension(_) => (None, None),
    }
}

fn app_url_path(url: &NSURL) -> Option<PathBuf> {
    url.path().map(|path| PathBuf::from(path.to_string()))
}

pub fn get_system_default_app(content_type: &str) -> String {
    autoreleasepool(|_| {
        let (default, _) = lookup_apps(content_type, false);
        default.as_deref().and_then(app_url_path)
            .and_then(|path| app_bundle(&path))
            .map(|(_, name)| name)
            .unwrap_or_default()
    })
}

pub fn get_associated_apps(extension: &str) -> Vec<Value> {
    autoreleasepool(|_| {
        let (default, associated) = lookup_apps(extension, true);
        let default = default.as_deref().and_then(app_url_path);
        let paths = associated.iter().flat_map(|apps| {
            (0..apps.count()).filter_map(|index| app_url_path(&apps.objectAtIndex(index)))
        });
        ordered_app_bundles(paths, default.as_deref())
    })
}

fn bundle_path_from_target(path: &Path) -> Option<PathBuf> {
    let path_str = path.to_string_lossy();
    if let Some(idx) = path_str.find(".app/") {
        return Some(PathBuf::from(&path_str[..idx + 4]));
    }

    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("app"))
        .unwrap_or(false)
    {
        return Some(path.to_path_buf());
    }

    None
}

fn normalize_icon_cache_key(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let candidate = Path::new(trimmed);
    if let Ok(resolved) = std::fs::canonicalize(candidate) {
        if let Some(bundle) = bundle_path_from_target(&resolved) {
            return bundle.to_string_lossy().to_string();
        }
        return resolved.to_string_lossy().to_string();
    }

    if let Some(bundle) = bundle_path_from_target(candidate) {
        return bundle.to_string_lossy().to_string();
    }

    trimmed.to_string()
}

fn normalize_file_icon_cache_key(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let candidate = Path::new(trimmed);
    if candidate.exists() {
        return normalize_icon_cache_key(trimmed);
    }

    candidate
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(|value| format!(".{}", value.to_ascii_lowercase()))
        .unwrap_or_else(|| trimmed.to_string())
}

#[cfg(target_os = "macos")]
fn image_to_data_url(image: &NSImage) -> Result<Option<String>, String> {
    autoreleasepool(|_| {
        let Some(tiff_data) = image.TIFFRepresentation() else {
            return Ok(None);
        };
        let Some(bitmap) = NSBitmapImageRep::imageRepWithData(&tiff_data) else {
            return Ok(None);
        };

        let properties = NSDictionary::<NSBitmapImageRepPropertyKey, AnyObject>::from_slices(
            &[] as &[&NSBitmapImageRepPropertyKey],
            &[] as &[&AnyObject],
        );

        let png_data = unsafe {
            bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
        };
        let Some(png_data) = png_data else {
            return Ok(None);
        };

        Ok(Some(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(shrink_icon_png(png_data.to_vec()))
        )))
    })
}

#[cfg(target_os = "macos")]
fn get_workspace_icon_for_file(path: &str) -> Result<Option<String>, String> {
    let normalized = normalize_icon_cache_key(path);
    if normalized.is_empty() {
        return Ok(None);
    }

    let candidate = Path::new(&normalized);
    if !candidate.exists() {
        return Ok(None);
    }

    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let ns_path = NSString::from_str(&normalized);
        let image = workspace.iconForFile(&ns_path);
        image_to_data_url(&image)
    })
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn get_workspace_icon_for_file_type(path_or_ext: &str) -> Result<Option<String>, String> {
    let key = normalize_file_icon_cache_key(path_or_ext);
    if key.is_empty() {
        return Ok(None);
    }

    let file_type = key.trim_start_matches('.').trim();
    if file_type.is_empty() {
        return Ok(None);
    }

    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let ns_type = NSString::from_str(file_type);
        let image = workspace.iconForFileType(&ns_type);
        image_to_data_url(&image)
    })
}

pub fn get_executable_icon(executable_path: String) -> Result<Option<String>, String> {
    let cache_key = normalize_icon_cache_key(&executable_path);
    if cache_key.is_empty() {
        return Ok(None);
    }

    let cache = EXECUTABLE_ICON_CACHE.get_or_init(|| Mutex::new(IconCache::default()));
    if let Some(cached) = cache.lock().map_err(|e| e.to_string())?.get(&cache_key) {
        return Ok(cached);
    }

    let icon = get_workspace_icon_for_file(&executable_path)?;
    cache
        .lock()
        .map_err(|e| e.to_string())?
        .insert(cache_key, icon.clone());
    Ok(icon)
}

pub fn get_file_icon(file_path: String) -> Result<Option<String>, String> {
    let cache_key = normalize_file_icon_cache_key(&file_path);
    if cache_key.is_empty() {
        return Ok(None);
    }

    let cache = FILE_ICON_CACHE.get_or_init(|| Mutex::new(IconCache::default()));
    if let Some(cached) = cache.lock().map_err(|e| e.to_string())?.get(&cache_key) {
        return Ok(cached);
    }

    let icon = if Path::new(file_path.trim()).exists() {
        get_workspace_icon_for_file(&file_path)?
    } else {
        get_workspace_icon_for_file_type(&file_path)?
    };

    cache
        .lock()
        .map_err(|e| e.to_string())?
        .insert(cache_key, icon.clone());
    Ok(icon)
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
pub fn activate_app_by_pid(pid: i32) -> bool {
    use objc2_app_kit::NSApplicationActivationOptions;
    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let apps = workspace.runningApplications();
        for i in 0..apps.count() {
            let app = apps.objectAtIndex(i);
            if app.processIdentifier() == pid {
                return app.activateWithOptions(
                    NSApplicationActivationOptions::ActivateIgnoringOtherApps,
                );
            }
        }
        false
    })
}

/// Soft activation for overlay show — restores keyboard without yanking windows.
#[cfg(target_os = "macos")]
#[allow(deprecated)]
pub fn activate_app_by_pid_soft(pid: i32) -> bool {
    use objc2_app_kit::NSApplicationActivationOptions;
    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let apps = workspace.runningApplications();
        for i in 0..apps.count() {
            let app = apps.objectAtIndex(i);
            if app.processIdentifier() == pid {
                // Empty options: prefer keyboard restore without forcing our
                // panel under other apps or stealing the next click.
                return app.activateWithOptions(NSApplicationActivationOptions::empty());
            }
        }
        false
    })
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
pub fn activate_app_by_name(name: &str) -> bool {
    use objc2_app_kit::NSApplicationActivationOptions;
    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let apps = workspace.runningApplications();
        let target_name = name.to_lowercase();
        for i in 0..apps.count() {
            let app = apps.objectAtIndex(i);
            if let Some(bundle_name) = app.localizedName().map(|s| s.to_string()) {
                if bundle_name.to_lowercase() == target_name {
                    return app.activateWithOptions(
                        NSApplicationActivationOptions::ActivateIgnoringOtherApps,
                    );
                }
            }
        }
        false
    })
}

#[cfg(test)]
mod association_tests {
    use super::{app_association, ordered_app_bundles, AppAssociation};
    use std::os::unix::fs::symlink;

    #[test]
    fn web_links_and_html_files_use_separate_associations() {
        for content_type in ["url", "link"] {
            assert_eq!(app_association(content_type), AppAssociation::WebLink);
        }
        for (content_type, ext) in [
            ("rich_text", "html"), (".html", "html"), ("text", "txt"), ("code", "txt"),
            ("image", "png"), ("video", "mp4"), (" .PDF ", "pdf"),
        ] {
            assert_eq!(app_association(content_type), AppAssociation::Extension(ext.into()));
        }
    }

    #[test]
    fn associations_use_existing_unique_bundles_with_default_first() {
        let root = std::env::temp_dir().join(format!("dewclip-apps-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        for name in ["z Default.app", "beta.APP", "Alpha.app", "other"] {
            std::fs::create_dir(root.join(name)).unwrap();
        }
        symlink(root.join("Alpha.app"), root.join("alias.app")).unwrap();
        let paths = ["beta.APP", "Alpha.app", "missing.app", "other", "alias.app"]
            .map(|name| root.join(name));
        let apps = ordered_app_bundles(paths, Some(&root.join("z Default.app")));
        let names: Vec<_> = apps.iter().map(|app| app["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["z Default", "Alpha", "beta"]);
        assert_eq!(
            apps[1]["path"], root.join("Alpha.app").canonicalize().unwrap().to_str().unwrap()
        );
        let default = root.join("z Default.app");
        let default_alias = root.join("default alias.app");
        symlink(&default, &default_alias).unwrap();
        assert_eq!(ordered_app_bundles([default_alias], Some(&default)).len(), 1);
        assert_eq!(ordered_app_bundles([], Some(&default))[0]["name"], "z Default");
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod icon_tests {
    use super::{shrink_icon_png, IconCache, ICON_CACHE_LIMIT, ICON_PIXELS};

    fn png(side: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::DynamicImage::new_rgba8(side, side)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn icons_are_sent_at_display_size() {
        let small = image::load_from_memory(&shrink_icon_png(png(512))).unwrap();
        assert_eq!((small.width(), small.height()), (ICON_PIXELS, ICON_PIXELS));
        let tiny = png(32);
        assert_eq!(shrink_icon_png(tiny.clone()), tiny);
        assert_eq!(shrink_icon_png(b"not a png".to_vec()), b"not a png".to_vec());
    }

    #[test]
    fn the_icon_cache_drops_the_oldest_past_its_limit() {
        let mut cache = IconCache::default();
        for index in 0..ICON_CACHE_LIMIT + 10 {
            cache.insert(format!("/file/{index}"), None);
        }
        assert_eq!(cache.icons.len(), ICON_CACHE_LIMIT);
        assert!(cache.get("/file/0").is_none());
        assert!(cache.get(&format!("/file/{}", ICON_CACHE_LIMIT + 9)).is_some());
        // Replacing a key does not count twice.
        cache.insert(format!("/file/{}", ICON_CACHE_LIMIT + 9), Some("icon".into()));
        assert_eq!(cache.order.len(), ICON_CACHE_LIMIT);
    }
}
