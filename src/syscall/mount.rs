use crate::filesystem::Filesystem;
use crate::syscall::Syscall;
use rustix::fs::{StatVfsMountFlags, stat, statvfs};
use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) enum MountSource {
    Bind(PathBuf),
    Fs(FsMountSource),
}

#[derive(Debug, Clone)]
pub(crate) struct FsMountSource {
    fs: Filesystem,
    path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MountOptions {
    readonly: bool,
    remount: bool,
}

impl MountSource {
    pub fn bind(path: impl AsRef<Path>) -> Self {
        Self::Bind(path.as_ref().to_path_buf())
    }

    pub fn fs(fs: Filesystem, path: impl AsRef<Path>) -> Self {
        Self::Fs(FsMountSource {
            fs,
            path: path.as_ref().to_path_buf(),
        })
    }
}

impl FsMountSource {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl MountOptions {
    pub fn readonly(mut self, readonly: bool) -> Self {
        self.readonly = readonly;
        self
    }

    pub fn remount(mut self, remount: bool) -> Self {
        self.remount = remount;
        self
    }

    fn flags(&self) -> MountFlags {
        let mut flags = MountFlags::empty();
        if self.readonly {
            flags |= MountFlags::RDONLY;
        }
        if self.remount {
            flags |= MountFlags::from_bits_truncate(linux_raw_sys::general::MS_REMOUNT);
        }
        flags
    }
}

impl Syscall {
    pub async fn same_file(
        &self,
        left: impl AsRef<Path>,
        right: impl AsRef<Path>,
    ) -> std::io::Result<bool> {
        let left = left.as_ref().to_path_buf();
        let right = right.as_ref().to_path_buf();
        Self::spawn(move || {
            let left = stat(left.as_path())?;
            let right = stat(right.as_path())?;
            Ok(left.st_dev == right.st_dev && left.st_ino == right.st_ino)
        })
        .await
    }

    pub async fn mount(
        &self,
        source: MountSource,
        target: impl AsRef<Path>,
        options: MountOptions,
    ) -> std::io::Result<()> {
        match source {
            MountSource::Bind(source) => self.mount_bind(source, target, options).await,
            MountSource::Fs(source) => self.mount_fs(source, target, options).await,
        }
    }

    async fn mount_fs(
        &self,
        source: FsMountSource,
        target: impl AsRef<Path>,
        options: MountOptions,
    ) -> std::io::Result<()> {
        let target = target.as_ref().to_path_buf();

        Self::spawn(move || match source.fs {
            Filesystem::Ext4 => Self::mount_ext4(source.path(), target, options),
            _ => Err(std::io::Error::new(
                ErrorKind::Unsupported,
                "Unsupported filesystem",
            )),
        })
        .await
    }

    async fn mount_bind(
        &self,
        source: impl AsRef<Path>,
        target: impl AsRef<Path>,
        options: MountOptions,
    ) -> std::io::Result<()> {
        let source = source.as_ref().to_path_buf();
        let target = target.as_ref().to_path_buf();

        Self::spawn(move || {
            Ok(mount(
                source,
                target,
                "",
                MountFlags::BIND | options.flags(),
                None,
            )?)
        })
        .await
    }

    fn mount_ext4(
        source: impl AsRef<Path>,
        target: impl AsRef<Path>,
        options: MountOptions,
    ) -> std::io::Result<()> {
        Ok(mount(
            source.as_ref(),
            target.as_ref(),
            "ext4",
            options.flags(),
            None,
        )?)
    }

    pub async fn unmount(&self, target: impl AsRef<Path>) -> std::io::Result<()> {
        let target = target.as_ref().to_path_buf();

        Self::spawn(move || Ok(unmount(target, UnmountFlags::empty())?)).await
    }

    /// Returns true when the filesystem mounted at `path` is mounted read-only.
    pub async fn is_readonly(&self, path: impl AsRef<Path>) -> std::io::Result<bool> {
        let path = path.as_ref().to_path_buf();

        Self::spawn(move || Ok(statvfs(path)?.f_flag.contains(StatVfsMountFlags::RDONLY))).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn same_file_compares_identity_not_path_text() {
        let root = std::env::temp_dir().join(format!(
            "loop-csi-same-file-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let left = root.join("left");
        let alias = root.join("alias");
        let right = root.join("right");
        tokio::fs::create_dir_all(&left).await.unwrap();
        tokio::fs::create_dir_all(&right).await.unwrap();
        tokio::fs::symlink(&left, &alias).await.unwrap();

        let syscall = Syscall::default();
        assert!(syscall.same_file(&left, &alias).await.unwrap());
        assert!(!syscall.same_file(&left, &right).await.unwrap());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
