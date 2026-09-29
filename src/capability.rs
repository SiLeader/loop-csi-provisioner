use crate::proto::csi::v1::VolumeCapability;

/// Returns true for the only kind of volume this driver serves: ext4 mount volumes
/// with single-node access and no mount flags or mount group.
pub(crate) fn supported_capability(capability: &VolumeCapability) -> bool {
    use crate::proto::csi::v1::volume_capability::{AccessType, access_mode::Mode};
    let mount_ok = matches!(&capability.access_type, Some(AccessType::Mount(mount))
        if (mount.fs_type.is_empty() || mount.fs_type == "ext4") && mount.mount_flags.is_empty() && mount.volume_mount_group.is_empty());
    let mode_ok = capability.access_mode.as_ref().is_some_and(|mode| {
        matches!(
            Mode::try_from(mode.mode),
            Ok(Mode::SingleNodeWriter | Mode::SingleNodeReaderOnly)
        )
    });
    mount_ok && mode_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::csi::v1::volume_capability::{
        AccessMode, AccessType, MountVolume, access_mode::Mode,
    };

    fn capability(fs_type: &str, mode: Mode) -> VolumeCapability {
        VolumeCapability {
            access_type: Some(AccessType::Mount(MountVolume {
                fs_type: fs_type.to_string(),
                ..Default::default()
            })),
            access_mode: Some(AccessMode { mode: mode as i32 }),
        }
    }

    #[test]
    fn accepts_only_ext4_single_node() {
        assert!(supported_capability(&capability(
            "ext4",
            Mode::SingleNodeWriter
        )));
        assert!(supported_capability(&capability(
            "",
            Mode::SingleNodeReaderOnly
        )));
        assert!(!supported_capability(&capability(
            "xfs",
            Mode::SingleNodeWriter
        )));
        assert!(!supported_capability(&capability(
            "ext4",
            Mode::MultiNodeMultiWriter
        )));
        assert!(!supported_capability(&VolumeCapability::default()));
    }
}
