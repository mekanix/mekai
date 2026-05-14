use std::path::{Path, PathBuf};

pub fn is_within_directory(child: &Path, parent: &Path) -> bool {
    let Ok(child) = child.canonicalize() else {
        return false;
    };
    let Ok(parent) = parent.canonicalize() else {
        return false;
    };
    child.starts_with(parent)
}

pub fn sanitize_path(path: &str) -> PathBuf {
    sanitize_filename::sanitize(path).into()
}
