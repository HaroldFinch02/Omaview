use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumConfig {
    pub albums: Vec<PathBuf>,
}

impl Default for AlbumConfig {
    fn default() -> Self {
        Self {
            albums: vec![default_pictures_dir()],
        }
    }
}

pub fn default_pictures_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(home).join("Pictures");
        if p.exists() {
            return p;
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        return cwd;
    }
    PathBuf::from(".")
}

pub fn config_path() -> PathBuf {
    if let Ok(config_home) = std::env::var("XDG_CONFIG_HOME") {
        let p = PathBuf::from(config_home).join("omaview/albums.toml");
        return p;
    }
    if let Ok(home) = std::env::var("HOME") {
        let p = PathBuf::from(home).join(".config/omaview/albums.toml");
        return p;
    }
    PathBuf::from("albums.toml")
}

pub fn load_albums() -> Vec<PathBuf> {
    let path = config_path();
    if let Some(albums) = read_saved_albums(&path) {
        return albums;
    }

    // Default configuration
    let default_cfg = AlbumConfig::default();
    let _ = save_albums(&default_cfg.albums);
    default_cfg.albums
}

fn read_saved_albums(path: &Path) -> Option<Vec<PathBuf>> {
    let content = std::fs::read_to_string(path).ok()?;
    Some(toml::from_str::<AlbumConfig>(&content).ok()?.albums)
}

pub fn save_albums(albums: &[PathBuf]) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create album config directory: {e}"))?;
    }

    let config = AlbumConfig {
        albums: albums.to_vec(),
    };

    let toml_str = toml::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize albums: {}", e))?;

    std::fs::write(&path, toml_str)
        .map_err(|e| format!("Failed to write albums to {}: {}", path.display(), e))
}

pub fn add_album(new_path: PathBuf) -> Result<Vec<PathBuf>, String> {
    let mut albums = load_albums();
    let canonical_new = std::fs::canonicalize(&new_path).unwrap_or_else(|_| new_path.clone());

    let already_present = albums.iter().any(|p| {
        let canonical_existing = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
        canonical_existing == canonical_new
    });

    if !already_present {
        albums.push(new_path);
        save_albums(&albums)?;
    }

    Ok(albums)
}

pub fn remove_album(to_remove: &Path) -> Result<Vec<PathBuf>, String> {
    let mut albums = load_albums();
    let canonical_target =
        std::fs::canonicalize(to_remove).unwrap_or_else(|_| to_remove.to_path_buf());

    albums.retain(|p| {
        let c = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
        c != canonical_target
    });

    save_albums(&albums)?;
    Ok(albums)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_saved_album_list_stays_empty() {
        let path =
            std::env::temp_dir().join(format!("omaview-empty-albums-{}.toml", std::process::id()));
        std::fs::write(&path, "albums = []").unwrap();
        assert_eq!(read_saved_albums(&path), Some(Vec::new()));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_default_pictures_dir() {
        let dir = default_pictures_dir();
        assert!(dir.exists() || dir == *".");
    }
}
