use std::path::Path;

const STORE_PLUGIN_MIRROR_BASE: &str =
    "https://github.com/shopware/store-plugin-mirror/blob/main/plugins/shopware6/plugin";

pub fn store_plugin_mirror_url(plugin_folder: &str, file_path: &Path, line: usize) -> String {
    format!(
        "{}/{}/{}#L{}",
        STORE_PLUGIN_MIRROR_BASE,
        encode_path_segment(plugin_folder),
        encode_path(file_path),
        line
    )
}

fn encode_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .split('/')
        .map(encode_path_segment)
        .collect::<Vec<_>>()
        .join("/")
}

fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::new();

    for byte in segment.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }

    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_store_plugin_mirror_line_url() {
        assert_eq!(
            store_plugin_mirror_url("AbiliAbilitaPay", Path::new("composer.json"), 6),
            "https://github.com/shopware/store-plugin-mirror/blob/main/plugins/shopware6/plugin/AbiliAbilitaPay/composer.json#L6"
        );
    }
}
