use crate::syscall::{Filesystem, Syscall};
use std::io::SeekFrom;
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

impl Syscall {
    pub async fn detect_filesystem(
        &self,
        file: impl AsRef<Path>,
    ) -> std::io::Result<Option<Filesystem>> {
        let mut f = File::open(file).await?;

        let mut buf = [0u8; 4];
        f.read_exact(&mut buf).await?;
        if &buf == b"XFSB" {
            return Ok(Some(Filesystem::Xfs));
        }

        f.seek(SeekFrom::Start(1024 + 0x38)).await?;
        let mut buf = [0u8; 2];
        f.read_exact(&mut buf).await?;
        if u16::from_le_bytes(buf) == 0xEF53 {
            Ok(Some(Filesystem::Ext4))
        } else {
            Ok(None)
        }
    }
}
