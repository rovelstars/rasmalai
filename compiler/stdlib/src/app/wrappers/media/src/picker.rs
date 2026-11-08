//! Native file picker. Headless uses `RNX_PICK_FILE`.

use rfd::FileDialog;

fn stub_value() -> Option<String> {
    std::env::var("RNX_PICK_FILE").ok()
}

fn split_paths(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn headless() -> bool {
    std::env::var_os("RNX_HEADLESS").is_some()
}

fn dialog(title: &str, filters: &str) -> FileDialog {
    let dialog = FileDialog::new().set_title(title);
    let extensions: Vec<String> = filters
        .split(',')
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.trim_start_matches('.').to_string())
        .collect();
    if extensions.is_empty() {
        dialog
    } else {
        dialog.add_filter("media", &extensions)
    }
}

pub fn pick_file(title: &str, filters: &str) -> Option<String> {
    if let Some(value) = stub_value() {
        return split_paths(&value).into_iter().next();
    }
    if headless() {
        return None;
    }
    dialog(title, filters)
        .pick_file()
        .map(|path| path.to_string_lossy().into_owned())
}

pub fn pick_files(title: &str, filters: &str) -> Vec<String> {
    if let Some(value) = stub_value() {
        return split_paths(&value);
    }
    if headless() {
        return Vec::new();
    }
    dialog(title, filters)
        .pick_files()
        .unwrap_or_default()
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn restore(key: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn stub_and_headless_paths() {
        let saved_file = std::env::var_os("RNX_PICK_FILE");
        let saved_headless = std::env::var_os("RNX_HEADLESS");

        std::env::set_var("RNX_PICK_FILE", "/tmp/one.png\n/tmp/two.mp4");
        assert_eq!(
            pick_file("Open", "png,mp4"),
            Some("/tmp/one.png".to_string())
        );
        assert_eq!(
            pick_files("Open", "png,mp4"),
            vec!["/tmp/one.png".to_string(), "/tmp/two.mp4".to_string()]
        );

        std::env::remove_var("RNX_PICK_FILE");
        std::env::set_var("RNX_HEADLESS", "1");
        assert_eq!(pick_file("Open", "png"), None);
        assert!(pick_files("Open", "png").is_empty());

        std::env::set_var("RNX_PICK_FILE", "");
        assert_eq!(pick_file("Open", "png"), None);
        assert!(pick_files("Open", "png").is_empty());

        restore("RNX_PICK_FILE", saved_file);
        restore("RNX_HEADLESS", saved_headless);
    }
}
