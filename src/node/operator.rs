use crate::filesystem::{Filesystem, FilesystemManager};
use crate::lock::KeyedLocks;
use crate::mount::MountManager;
use crate::node::error::NodeError;
use crate::syscall::{MountOptions, MountSource, Syscall};
use crate::volume_id::{VOLUME_DIR, parse_volume_id};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::warn;

pub(crate) struct NodeOperator {
    base_directory: String,
    mounter: Arc<MountManager>,
    syscall: Syscall,
    fs: FilesystemManager,
    locks: KeyedLocks,
}

impl NodeOperator {
    pub async fn new(base_directory: String, mounter: Arc<MountManager>) -> std::io::Result<Self> {
        Ok(Self {
            base_directory,
            mounter,
            syscall: Syscall::default(),
            fs: FilesystemManager::default(),
            locks: KeyedLocks::default(),
        })
    }

    async fn volume_file_path(&self, volume_id: &str) -> Result<PathBuf, NodeError> {
        let (url, volume_id) = parse_volume_id(volume_id).ok_or(NodeError::VolumeIdParse)?;

        let mount_point = self.mounter.mount(url, &self.base_directory).await?;
        let storage_base_directory = PathBuf::from(&mount_point).join(VOLUME_DIR);
        Ok(storage_base_directory.join(format!("{}.img", volume_id)))
    }

    /// Returns the loop device already attached to the volume's image, if any.
    async fn attached_loop_device(&self, volume_id: &str) -> Result<Option<String>, NodeError> {
        let path = self.volume_file_path(volume_id).await?;
        if !tokio::fs::try_exists(&path).await? {
            return Err(NodeError::NotFound(volume_id.to_string()));
        }
        let device = self.syscall.resolve_attached_loop_device(&path).await?;
        Ok(device.map(|d| d.to_string_lossy().to_string()))
    }

    /// Returns the loop device for the volume, attaching the image if needed.
    /// The flag tells whether this call attached it.
    async fn ensure_loop_device(&self, volume_id: &str) -> Result<(String, bool), NodeError> {
        if let Some(device) = self.attached_loop_device(volume_id).await? {
            return Ok((device, false));
        }
        let path = self.volume_file_path(volume_id).await?;
        let device = self.syscall.find_loop(&path).await?;
        Ok((device.to_string_lossy().to_string(), true))
    }

    /// Attaches, formats if blank, and mounts the volume. A `read_only` volume is mounted
    /// read-only and never formatted.
    pub async fn stage_volume(
        &self,
        volume_id: &str,
        staging_target_path: &str,
        read_only: bool,
    ) -> Result<(), NodeError> {
        let _guard = self.locks.lock(volume_id).await;

        if self.is_mount_point(staging_target_path).await? {
            self.verify_staged_volume(volume_id, staging_target_path)
                .await?;
            if self.syscall.is_readonly(staging_target_path).await? != read_only {
                return Err(NodeError::WrongMountAt(staging_target_path.to_string()));
            }
            return Ok(());
        }
        tokio::fs::create_dir_all(staging_target_path).await?;
        let (loop_device, newly_attached) = self.ensure_loop_device(volume_id).await?;

        match self
            .format_and_mount(&loop_device, staging_target_path, read_only)
            .await
        {
            Ok(()) => Ok(()),
            Err(e) => {
                // Do not leak a device this call attached; a retry attaches a fresh one.
                if newly_attached
                    && let Err(detach_err) = self.syscall.detach_loop(&loop_device).await
                {
                    warn!(device = %loop_device, error = %detach_err, "failed to detach loop device after failed stage");
                }
                Err(e)
            }
        }
    }

    async fn format_and_mount(
        &self,
        loop_device: &str,
        staging_target_path: &str,
        read_only: bool,
    ) -> Result<(), NodeError> {
        let fs = match self.syscall.detect_filesystem(loop_device).await? {
            None if read_only => {
                return Err(NodeError::ReadOnlyUnformatted(loop_device.to_string()));
            }
            None => {
                // Only format a device that is entirely unused; anything else may be someone's data.
                if !self.syscall.is_blank(loop_device).await? {
                    return Err(NodeError::NotBlank(loop_device.to_string()));
                }
                self.fs.create(Filesystem::Ext4, loop_device).await?
            }
            Some(Filesystem::Ext4) => Filesystem::Ext4,
            Some(_) => return Err(NodeError::UnsupportedFilesystem),
        };

        self.syscall
            .mount(
                MountSource::fs(fs, loop_device),
                staging_target_path,
                MountOptions::default().readonly(read_only),
            )
            .await?;
        Ok(())
    }

    pub async fn publish_volume(
        &self,
        volume_id: &str,
        staging_target_path: &str,
        target_path: &str,
        readonly: bool,
    ) -> Result<(), NodeError> {
        let _guard = self.locks.lock(target_path).await;

        self.verify_staged_volume(volume_id, staging_target_path)
            .await?;
        tokio::fs::create_dir_all(target_path).await?;

        let src = MountSource::bind(staging_target_path);
        let remount_readonly = MountOptions::default().remount(true).readonly(true);
        if self.is_mount_point(target_path).await? {
            if !self
                .syscall
                .same_file(staging_target_path, target_path)
                .await?
            {
                return Err(NodeError::WrongMountAt(target_path.to_string()));
            }
            // A previous attempt may have bound the volume but failed to make it read-only.
            if readonly && !self.syscall.is_readonly(target_path).await? {
                self.syscall
                    .mount(src, target_path, remount_readonly)
                    .await?;
            } else if !readonly && self.syscall.is_readonly(target_path).await? {
                return Err(NodeError::WrongMountAt(target_path.to_string()));
            }
            return Ok(());
        }

        self.syscall
            .mount(src.clone(), target_path, MountOptions::default())
            .await?;
        if readonly && let Err(e) = self.syscall.mount(src, target_path, remount_readonly).await {
            // Never leave a writable mount behind for a read-only request.
            if let Err(unmount_err) = self.syscall.unmount(target_path).await {
                warn!(target = %target_path, error = %unmount_err, "failed to unmount after failed read-only remount");
            }
            return Err(e.into());
        }
        Ok(())
    }

    async fn is_mount_point(&self, path: &str) -> Result<bool, NodeError> {
        let res = self.mounter.check_mount_point(path).await?;
        Ok(res)
    }

    async fn verify_staged_volume(&self, volume_id: &str, path: &str) -> Result<(), NodeError> {
        if !self.is_mount_point(path).await? {
            return Err(NodeError::NotStagedAt(path.to_string()));
        }
        let Some(device) = self.syscall.loop_device_of_mount(path).await? else {
            return Err(NodeError::WrongVolumeAt(path.to_string()));
        };
        let image = self.volume_file_path(volume_id).await?;
        if !self
            .syscall
            .loop_device_matches_image(device, image)
            .await?
        {
            return Err(NodeError::WrongVolumeAt(path.to_string()));
        }
        Ok(())
    }

    pub async fn unpublish_volume(&self, target_path: &str) -> Result<(), NodeError> {
        let _guard = self.locks.lock(target_path).await;

        if self.is_mount_point(target_path).await? {
            self.syscall.unmount(target_path).await?;
        }
        remove_dir_best_effort(target_path).await;
        Ok(())
    }

    pub async fn unstage_volume(
        &self,
        volume_id: &str,
        staging_target_path: &str,
    ) -> Result<(), NodeError> {
        let _guard = self.locks.lock(volume_id).await;

        // Identify the mounted loop device before unmounting, and verify its backing image.
        let mut device = None;
        if self.is_mount_point(staging_target_path).await? {
            self.verify_staged_volume(volume_id, staging_target_path)
                .await?;
            device = self
                .syscall
                .loop_device_of_mount(staging_target_path)
                .await?;
            self.syscall.unmount(staging_target_path).await?;
        }
        let device = match device {
            Some(device) => Some(device.to_string_lossy().to_string()),
            None => match self.attached_loop_device(volume_id).await {
                Ok(device) => device,
                Err(NodeError::NotFound(_)) => None,
                Err(e) => return Err(e),
            },
        };
        if let Some(device) = device {
            self.syscall.detach_loop(&device).await?;
        }
        remove_dir_best_effort(staging_target_path).await;
        Ok(())
    }

    /// Grows the filesystem to the size of the image and returns the new capacity.
    /// `required` is the size the CO expects, or 0 when it did not say.
    pub async fn expand_volume(&self, volume_id: &str, required: i64) -> Result<i64, NodeError> {
        let _guard = self.locks.lock(volume_id).await;

        // The volume is mounted through an existing loop device; that device (not a new one)
        // must learn about the new file size before the filesystem is grown.
        let loop_device = self
            .attached_loop_device(volume_id)
            .await?
            .ok_or_else(|| NodeError::NotStaged(volume_id.to_string()))?;
        let path = self.volume_file_path(volume_id).await?;
        let device_size = self
            .syscall
            .refresh_loop_device_capacity(&loop_device, &path)
            .await?;
        // Loop devices ignore a trailing partial sector of the image.
        let image_size = tokio::fs::metadata(&path).await?.len() & !511;
        let expected = image_size.max(u64::try_from(required).unwrap_or(0));
        if device_size < expected {
            // Growing the filesystem now would silently keep the old size.
            return Err(NodeError::CapacityNotVisible {
                device: loop_device,
                actual: device_size,
                expected,
            });
        }

        self.fs.resize(Filesystem::Ext4, &loop_device).await?;
        Ok(device_size as i64)
    }
}

/// Removes the directory created for a (un)staged or (un)published path.
/// CSI requires the plugin to delete what it created, but a leftover must not fail the RPC.
async fn remove_dir_best_effort(path: &str) {
    match tokio::fs::remove_dir(path).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => warn!(path = %path, error = %e, "failed to remove directory"),
    }
}
