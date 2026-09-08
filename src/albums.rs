use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

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
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(config) = toml::from_str::<AlbumConfig>(&content) {
                if !config.albums.is_empty() {
                    return config.albums;
                }
            }
        }
    }

    // Default configuration
    let default_cfg = AlbumConfig::default();
    let _ = save_albums(&default_cfg.albums);
    default_cfg.albums
}

pub fn save_albums(albums: &[PathBuf]) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
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
    let canonical_target = std::fs::canonicalize(to_remove).unwrap_or_else(|_| to_remove.to_path_buf());

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
    fn test_default_pictures_dir() {
        let dir = default_pictures_dir();
        assert!(dir.exists() || dir == PathBuf::from("."));
    }
}
