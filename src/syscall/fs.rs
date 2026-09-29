use crate::filesystem::Filesystem;
use crate::syscall::Syscall;
use std::io::{ErrorKind, SeekFrom};
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

/// How much of the start of a device must be zero for it to count as unused.
/// Covers the signature areas of ext*, XFS, btrfs (64 KiB), LUKS, and partition tables.
const BLANK_CHECK_BYTES: usize = 1024 * 1024;

impl Syscall {
    pub async fn detect_filesystem(
        &self,
        file: impl AsRef<Path>,
    ) -> std::io::Result<Option<Filesystem>> {
        let mut f = File::open(file).await?;

        let mut buf = [0u8; 4];
        match f.read_exact(&mut buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        if &buf == b"XFSB" {
            return Ok(Some(Filesystem::Xfs));
        }

        f.seek(SeekFrom::Start(1024 + 0x38)).await?;
        let mut buf = [0u8; 2];
        match f.read_exact(&mut buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        if u16::from_le_bytes(buf) == 0xEF53 {
            Ok(Some(Filesystem::Ext4))
        } else {
            Ok(None)
        }
    }

    /// Returns true when the first [`BLANK_CHECK_BYTES`] of `file` (or all of it, if shorter)
    /// are zero, i.e. it is safe to format.
    pub async fn is_blank(&self, file: impl AsRef<Path>) -> std::io::Result<bool> {
        let mut f = File::open(file).await?;
        let mut remaining = BLANK_CHECK_BYTES;
        let mut buf = vec![0u8; 64 * 1024];
        while remaining > 0 {
            let want = remaining.min(buf.len());
            let n = f.read(&mut buf[..want]).await?;
            if n == 0 {
                break;
            }
            if buf[..n].iter().any(|b| *b != 0) {
                return Ok(false);
            }
            remaining -= n;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn temp_file(name: &str, content: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("loop-csi-{name}-{}", std::process::id()));
        tokio::fs::write(&path, content).await.unwrap();
        path
    }

    #[tokio::test]
    async fn blank_detection() {
        let sys = Syscall::default();
        let zeros = temp_file("zeros", &vec![0u8; 2 * 1024 * 1024]).await;
        assert!(sys.is_blank(&zeros).await.unwrap());

        let mut data = vec![0u8; 2 * 1024 * 1024];
        data[70_000] = 1; // e.g. a btrfs signature area
        let dirty = temp_file("dirty", &data).await;
        assert!(!sys.is_blank(&dirty).await.unwrap());

        let empty = temp_file("empty", b"").await;
        assert!(sys.is_blank(&empty).await.unwrap());
        for p in [zeros, dirty, empty] {
            tokio::fs::remove_file(p).await.unwrap();
        }
    }

    #[tokio::test]
    async fn filesystem_detection() {
        let sys = Syscall::default();
        let mut ext = vec![0u8; 4096];
        ext[1024 + 0x38..1024 + 0x3a].copy_from_slice(&0xEF53u16.to_le_bytes());
        let ext_file = temp_file("ext", &ext).await;
        assert_eq!(
            sys.detect_filesystem(&ext_file).await.unwrap(),
            Some(Filesystem::Ext4)
        );

        let mut xfs = vec![0u8; 4096];
        xfs[..4].copy_from_slice(b"XFSB");
        let xfs_file = temp_file("xfs", &xfs).await;
        assert_eq!(
            sys.detect_filesystem(&xfs_file).await.unwrap(),
            Some(Filesystem::Xfs)
        );

        let blank = temp_file("blank-fs", &vec![0u8; 4096]).await;
        assert_eq!(sys.detect_filesystem(&blank).await.unwrap(), None);
        let tiny = temp_file("tiny", b"ab").await;
        assert_eq!(sys.detect_filesystem(&tiny).await.unwrap(), None);
        for p in [ext_file, xfs_file, blank, tiny] {
            tokio::fs::remove_file(p).await.unwrap();
        }
    }
}
