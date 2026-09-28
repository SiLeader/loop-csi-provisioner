pub(crate) const VOLUME_DIR: &str = "volumes";
pub(crate) const METADATA_DIR: &str = "metadata";

pub(crate) fn parse_volume_id(volume_id: &str) -> Option<(&str, &str)> {
    let (url, name) = volume_id.rsplit_once(':')?;
    if !url.contains("://") || !valid_volume_name(name) {
        return None;
    }
    Some((url, name))
}

pub(crate) fn valid_volume_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_ids() {
        assert_eq!(
            parse_volume_id("file:///data:pvc-1"),
            Some(("file:///data", "pvc-1"))
        );
        for id in [
            "file:///data:../secret",
            "file:///data:",
            "file:///data:a/b",
            "plain",
        ] {
            assert!(parse_volume_id(id).is_none());
        }
    }
}
