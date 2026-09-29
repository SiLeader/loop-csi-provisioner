use crate::filesystem::Filesystem;
use crate::syscall::Syscall;
use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) enum MountSource {
    Bind(PathBuf),
    Nfs(PathBuf),
    Fs(FsMountSource),
}

#[derive(Debug, Clone)]
pub(crate) struct FsMountSource {
    fs: Filesystem,
    path: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct MountOptions {
    readonly: bool,
    remount: bool,
}

impl MountSource {
    pub fn bind(path: impl AsRef<Path>) -> Self {
        Self::Bind(path.as_ref().to_path_buf())
    }

    pub fn nfs(path: impl AsRef<Path>) -> Self {
        Self::Nfs(path.as_ref().to_path_buf())
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

impl Default for MountOptions {
    fn default() -> Self {
        Self {
            readonly: false,
            remount: false,
        }
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
    pub async fn mount(
        &self,
        source: MountSource,
        target: impl AsRef<Path>,
        options: MountOptions,
    ) -> std::io::Result<()> {
        match source {
            MountSource::Bind(source) => self.mount_bind(source, target, options).await,
            MountSource::Nfs(source) => self.mount_nfs(source, target, options).await,
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

    async fn mount_nfs(
        &self,
        source: impl AsRef<Path>,
        target: impl AsRef<Path>,
        options: MountOptions,
    ) -> std::io::Result<()> {
        let source = source.as_ref().to_path_buf();
        let target = target.as_ref().to_path_buf();

        Self::spawn(move || Ok(mount(source, target, "nfs", options.flags(), None)?)).await
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
}
