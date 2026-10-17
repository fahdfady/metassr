//! Mapping between page source paths and manifest route keys.

use std::path::Path;

/// Maps a root-relative page source path to the route key used in
/// `dist/manifest.json` and by the dist directory analyzer.
///
/// - `src/pages/index.tsx`         -> `#root`
/// - `src/pages/home.tsx`          -> `home`
/// - `src/pages/blog/index.tsx`    -> `blog`
/// - `src/pages/blog/$article.tsx` -> `blog/$article`
///
/// Returns `None` when the path is outside `src/pages` or has no route
/// (e.g. the pages directory itself).
pub fn page_source_to_route(path: &Path) -> Option<String> {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let relative = normalized.strip_prefix("src/pages")?;

    // Reject paths that merely share the prefix (e.g. `src/pages.bak/...`).
    if !relative.is_empty() && !relative.starts_with('/') {
        return None;
    }

    let relative = relative.strip_prefix('/').unwrap_or(relative);
    if relative.is_empty() {
        return None;
    }

    let without_extension = match relative.rfind('.') {
        Some(index) if !relative[index + 1..].contains('/') => &relative[..index],
        _ => relative,
    };

    if without_extension.is_empty() {
        return None;
    }

    if without_extension == "index" {
        return Some("#root".to_string());
    }

    let route = without_extension
        .strip_suffix("/index")
        .unwrap_or(without_extension);

    if route.is_empty() {
        return None;
    }

    Some(route.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_root_index_to_hash_root() {
        assert_eq!(
            page_source_to_route(Path::new("src/pages/index.tsx")),
            Some("#root".to_string())
        );
    }

    #[test]
    fn maps_plain_page() {
        assert_eq!(
            page_source_to_route(Path::new("src/pages/home.tsx")),
            Some("home".to_string())
        );
    }

    #[test]
    fn maps_nested_index_to_directory_route() {
        assert_eq!(
            page_source_to_route(Path::new("src/pages/blog/index.tsx")),
            Some("blog".to_string())
        );
    }

    #[test]
    fn keeps_dynamic_segments() {
        assert_eq!(
            page_source_to_route(Path::new("src/pages/blog/$article.tsx")),
            Some("blog/$article".to_string())
        );
    }

    #[test]
    fn handles_dots_in_directory_names() {
        assert_eq!(
            page_source_to_route(Path::new("src/pages/v1.2/details.tsx")),
            Some("v1.2/details".to_string())
        );
    }

    #[test]
    fn handles_windows_separators() {
        assert_eq!(
            page_source_to_route(Path::new(r"src\pages\blog\$article.tsx")),
            Some("blog/$article".to_string())
        );
    }

    #[test]
    fn rejects_paths_outside_pages() {
        assert_eq!(page_source_to_route(Path::new("src/api/hello.js")), None);
        assert_eq!(page_source_to_route(Path::new("src/pages.bak/x.tsx")), None);
        assert_eq!(page_source_to_route(Path::new("src/pages")), None);
    }
}
