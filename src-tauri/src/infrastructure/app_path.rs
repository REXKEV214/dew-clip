use std::borrow::Cow;
use std::path::{Path, PathBuf};

pub(crate) fn canonical_app_path(path: &Path) -> Option<PathBuf> {
    let canonical = path.canonicalize().ok()?;
    (canonical.extension()?.eq_ignore_ascii_case("app") && canonical.is_dir()).then_some(canonical)
}

pub(crate) fn normalize_app_path(path: &str) -> Cow<'_, str> {
    let literal = path.trim().trim_end_matches('/');
    let candidate = Path::new(literal);
    if candidate.is_absolute()
        && candidate
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
    {
        if let Some(canonical) =
            canonical_app_path(candidate).and_then(|path| path.into_os_string().into_string().ok())
        {
            return Cow::Owned(canonical);
        }
    }
    Cow::Borrowed(literal)
}
