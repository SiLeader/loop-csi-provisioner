pub(crate) const VOLUME_DIR: &str = "volumes";
pub(crate) const METADATA_DIR: &str = "metadata";

pub(crate) fn parse_volume_id(volume_id: &str) -> Option<(&str, &str)> {
    volume_id.rsplit_once(':')
}
