use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct RawEditorInfo {
    pub binary: String,
    pub display_name: String,
}

static DETECTED_RAW_EDITOR: OnceLock<Option<RawEditorInfo>> = OnceLock::new();

pub fn detect_raw_editor() -> Option<RawEditorInfo> {
    DETECTED_RAW_EDITOR
        .get_or_init(|| {
            let editors = [
                ("darktable", "Darktable"),
                ("digikam", "digiKam"),
                ("rawtherapee", "RawTherapee"),
                ("gimp", "GIMP"),
            ];

            for (bin, name) in editors {
                if check_binary_in_path(bin).is_ok() {
                    return Some(RawEditorInfo {
                        binary: bin.to_string(),
                        display_name: name.to_string(),
                    });
                }
            }

            if check_binary_in_path("xdg-open").is_ok() {
                Some(RawEditorInfo {
                    binary: "xdg-open".to_string(),
                    display_name: "System RAW Viewer".to_string(),
                })
            } else {
                None
            }
        })
        .clone()
}

fn check_binary_in_path(bin: &str) -> Result<PathBuf, ()> {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(bin);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(())
}

pub fn launch_in_raw_editor(path: &Path) -> Result<String, String> {
    let editor = detect_raw_editor().ok_or_else(|| "No RAW editor or xdg-open found on system".to_string())?;
    Command::new(&editor.binary)
        .arg(path)
        .spawn()
        .map_err(|e| format!("Failed to launch {}: {}", editor.display_name, e))?;
    Ok(editor.display_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_binary_in_path() {
        assert!(check_binary_in_path("sh").is_ok());
        assert!(check_binary_in_path("non_existent_binary_xyz_123").is_err());
    }

    #[test]
    fn test_detect_raw_editor() {
        let editor = detect_raw_editor();
        // xdg-open exists on standard linux desktop systems
        if check_binary_in_path("xdg-open").is_ok() {
            assert!(editor.is_some());
            let ed = editor.unwrap();
            assert!(!ed.binary.is_empty());
            assert!(!ed.display_name.is_empty());
        }
    }
}
