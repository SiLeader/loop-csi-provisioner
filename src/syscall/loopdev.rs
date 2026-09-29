use crate::syscall::Syscall;
use rustix::fs::{
    CWD, FileType, Mode, OFlags, SeekFrom, fstat, major, makedev, minor, mknodat, open, seek, stat,
};
use rustix::io::Errno;
use rustix::ioctl::{Getter, Ioctl, IoctlOutput, NoArg, Opcode, Setter, ioctl};
use std::ffi::c_void;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};

/// Returns `/dev/<name>` for the block device `name`, creating the device node if missing.
///
/// In a container whose `/dev` is a copy made at start-up (e.g. a kind node), loop devices
/// the kernel creates later have no node, so it is made from the numbers sysfs reports.
fn device_node(name: &str) -> std::io::Result<PathBuf> {
    let path = Path::new("/dev").join(name);
    if path.exists() {
        return Ok(path);
    }
    let numbers = std::fs::read_to_string(Path::new("/sys/block").join(name).join("dev"))?;
    let (major, minor) = parse_dev_numbers(&numbers).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("unexpected device numbers for {name}: {numbers:?}"),
        )
    })?;
    match mknodat(
        CWD,
        &path,
        FileType::BlockDevice,
        Mode::from_raw_mode(0o660),
        makedev(major, minor),
    ) {
        Ok(()) | Err(Errno::EXIST) => Ok(path),
        Err(e) => Err(e.into()),
    }
}

/// Parses the `MAJOR:MINOR` format of sysfs `dev` files.
fn parse_dev_numbers(value: &str) -> Option<(u32, u32)> {
    let (major, minor) = value.trim().split_once(':')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Returns true when the sysfs directory of a block device describes a bound loop device.
fn is_bound_loop(sys_dir: &Path) -> bool {
    sys_dir.join("loop").join("backing_file").exists()
}

impl Syscall {
    /// Attaches `file` to a free loop device and returns the device path.
    ///
    /// A free device number can be taken by another process between `LOOP_CTL_GET_FREE`
    /// and `LOOP_CONFIGURE`, so `EBUSY` is retried with a fresh number.
    pub async fn find_loop(&self, file: impl AsRef<Path>) -> std::io::Result<PathBuf> {
        let file = file.as_ref().to_path_buf();

        Self::spawn(move || {
            let image_fd = open(
                file.as_path(),
                OFlags::RDWR | OFlags::CLOEXEC,
                Mode::empty(),
            )?;
            let lc = LoopControl::open()?;
            let mut attempt = 1;
            loop {
                let ld = lc.next_free()?;
                match ld.attach_file(&image_fd) {
                    Ok(()) => return Ok(ld.into_path()),
                    Err(e)
                        if e.raw_os_error() == Some(Errno::BUSY.raw_os_error())
                            && attempt < ATTACH_ATTEMPTS =>
                    {
                        attempt += 1;
                    }
                    Err(e) => return Err(e),
                }
            }
        })
        .await
    }

    /// Returns the loop device backing the filesystem mounted at `path`, if any.
    pub async fn loop_device_of_mount(
        &self,
        path: impl AsRef<Path>,
    ) -> std::io::Result<Option<PathBuf>> {
        let path = path.as_ref().to_path_buf();
        Self::spawn(move || {
            let dev = stat(path.as_path())?.st_dev;
            // Look the device up instead of deriving `loopN` from the minor number, which
            // does not hold when the loop driver reserves minors for partitions (max_part).
            let sys_dir = PathBuf::from(format!("/sys/dev/block/{}:{}", major(dev), minor(dev)));
            if !is_bound_loop(&sys_dir) {
                return Ok(None);
            }
            let target = std::fs::read_link(&sys_dir)?;
            let Some(name) = target.file_name().and_then(|n| n.to_str()) else {
                return Ok(None);
            };
            Ok(Some(device_node(name)?))
        })
        .await
    }

    /// Makes the loop device pick up the current size of `image` and returns the device's
    /// new size in bytes.
    pub async fn refresh_loop_device_capacity(
        &self,
        loop_device: impl AsRef<Path>,
        image: impl AsRef<Path>,
    ) -> std::io::Result<u64> {
        let ld = loop_device.as_ref().to_path_buf();
        let image = image.as_ref().to_path_buf();
        Self::spawn(move || {
            // LOOP_SET_CAPACITY uses the size the kernel has cached for the image's inode.
            // Opening the image by path makes an NFS client revalidate that cache
            // (close-to-open consistency), so a resize done by the controller is seen.
            let image = open(
                image.as_path(),
                OFlags::RDONLY | OFlags::CLOEXEC,
                Mode::empty(),
            )?;
            fstat(&image)?;
            let ld = LoopDevice::open(ld)?;
            ld.apply_capacity()?;
            ld.size()
        })
        .await
    }

    pub async fn resolve_attached_loop_device(
        &self,
        image_path: impl AsRef<Path>,
    ) -> std::io::Result<Option<PathBuf>> {
        get_attached_loop_device(image_path).await
    }

    pub async fn detach_loop(&self, loop_device: impl AsRef<Path>) -> std::io::Result<()> {
        let ld = loop_device.as_ref().to_path_buf();
        Self::spawn(move || {
            let ld = LoopDevice::open(ld)?;
            ld.detach()
        })
        .await
    }
}

async fn get_attached_loop_device(
    image_file: impl AsRef<Path>,
) -> std::io::Result<Option<PathBuf>> {
    let image_file = image_file.as_ref().to_path_buf();
    let image_stat = Syscall::spawn(move || Ok(stat(image_file.as_path())?)).await?;

    let mut dir = tokio::fs::read_dir("/sys/block").await?;
    while let Some(entry) = dir.next_entry().await? {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };

        // Unbound loop devices have no backing file to compare, so skip them early.
        if !name.starts_with("loop") || !is_bound_loop(&entry.path()) {
            continue;
        }
        let name = name.to_string();

        let info = Syscall::spawn(move || {
            let Ok(device) = device_node(&name) else {
                return Ok(None);
            };
            let Ok(ld) = LoopDevice::open(device.clone()) else {
                return Ok(None);
            };
            let Ok(info) = ld.get_status() else {
                return Ok(None);
            };
            Ok(Some((device, info)))
        })
        .await?;
        if let Some((device, info)) = info
            && info.lo_device == image_stat.st_dev
            && info.lo_inode == image_stat.st_ino
        {
            return Ok(Some(device));
        }
    }
    Ok(None)
}

struct LoopControl {
    file: OwnedFd,
}

struct LoopDevice {
    path: PathBuf,
    file: OwnedFd,
}

const LOOP_CTL_GET_FREE: Opcode = linux_raw_sys::loop_device::LOOP_CTL_GET_FREE;
const LOOP_CONFIGURE: Opcode = linux_raw_sys::loop_device::LOOP_CONFIGURE;
const LOOP_CLR_FD: Opcode = linux_raw_sys::loop_device::LOOP_CLR_FD;
const LOOP_SET_CAPACITY: Opcode = linux_raw_sys::loop_device::LOOP_SET_CAPACITY;
const LOOP_GET_STATUS64: Opcode = linux_raw_sys::loop_device::LOOP_GET_STATUS64;

/// How many times attaching is retried when a concurrent caller grabs the same free device.
const ATTACH_ATTEMPTS: u32 = 16;

#[repr(C)]
struct LoopInfo64 {
    lo_device: u64,
    lo_inode: u64,
    lo_rdevice: u64,
    lo_offset: u64,
    lo_sizelimit: u64,
    lo_number: u32,
    lo_encrypt_type: u32,
    lo_encrypt_key_size: u32,
    lo_flags: u32,
    lo_file_name: [u8; 64],
    lo_crypt_name: [u8; 64],
    lo_encrypt_key: [u8; 32],
    lo_init: [u64; 2],
}

#[repr(C)]
#[derive(Default)]
struct LoopConfig {
    fd: u32,
    block_size: u32,
    info: LoopInfo64,
    reserved: [u64; 8],
}

struct RetValue<const OPCODE: Opcode> {}

impl<const OPCODE: Opcode> RetValue<OPCODE> {
    fn new() -> Self {
        Self {}
    }
}

impl LoopControl {
    fn open() -> std::io::Result<Self> {
        let file = open(
            "/dev/loop-control",
            OFlags::RDWR | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        Ok(Self { file })
    }

    fn next_free(&self) -> std::io::Result<LoopDevice> {
        let num = unsafe {
            let ctl = RetValue::<LOOP_CTL_GET_FREE>::new();
            ioctl(&self.file, ctl)?
        };
        if num < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            LoopDevice::open(device_node(&format!("loop{num}"))?)
        }
    }
}

impl LoopDevice {
    fn open(path: PathBuf) -> std::io::Result<Self> {
        let loop_fd = open(&path, OFlags::RDWR | OFlags::CLOEXEC, Mode::empty())?;
        Ok(Self {
            path,
            file: loop_fd,
        })
    }

    fn attach_file(&self, image_fd: &OwnedFd) -> std::io::Result<()> {
        let config = LoopConfig {
            fd: image_fd.as_raw_fd() as u32,
            block_size: 0,
            ..Default::default()
        };

        unsafe {
            // Setter passes a pointer to its value, so the value must be the struct itself.
            let ctl = Setter::<LOOP_CONFIGURE, LoopConfig>::new(config);
            ioctl(&self.file, ctl)?;
        }

        Ok(())
    }

    fn detach(&self) -> std::io::Result<()> {
        unsafe {
            ioctl(&self.file, NoArg::<LOOP_CLR_FD>::new())?;
        }
        Ok(())
    }

    fn apply_capacity(&self) -> std::io::Result<()> {
        unsafe {
            ioctl(&self.file, NoArg::<LOOP_SET_CAPACITY>::new())?;
        }
        Ok(())
    }

    fn size(&self) -> std::io::Result<u64> {
        Ok(seek(&self.file, SeekFrom::End(0))?)
    }

    fn get_status(&self) -> std::io::Result<LoopInfo64> {
        let info = unsafe { ioctl(&self.file, Getter::<LOOP_GET_STATUS64, LoopInfo64>::new())? };
        Ok(info)
    }

    fn into_path(self) -> PathBuf {
        self.path
    }
}

impl Default for LoopInfo64 {
    fn default() -> Self {
        Self {
            lo_device: 0,
            lo_inode: 0,
            lo_rdevice: 0,
            lo_offset: 0,
            lo_sizelimit: 0,
            lo_number: 0,
            lo_encrypt_type: 0,
            lo_encrypt_key_size: 0,
            lo_flags: 0,
            lo_file_name: [0u8; 64],
            lo_crypt_name: [0u8; 64],
            lo_encrypt_key: [0u8; 32],
            lo_init: [0u64; 2],
        }
    }
}

unsafe impl<const OPCODE: Opcode> Ioctl for RetValue<OPCODE> {
    type Output = i32;
    const IS_MUTATING: bool = false;

    fn opcode(&self) -> Opcode {
        OPCODE
    }

    fn as_ptr(&mut self) -> *mut c_void {
        std::ptr::null_mut()
    }

    unsafe fn output_from_ptr(
        out: IoctlOutput,
        _extract_output: *mut c_void,
    ) -> rustix::io::Result<Self::Output> {
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sysfs_device_numbers() {
        assert_eq!(parse_dev_numbers("7:3\n"), Some((7, 3)));
        assert_eq!(parse_dev_numbers("259:1048576"), Some((259, 1048576)));
        assert_eq!(parse_dev_numbers("7"), None);
        assert_eq!(parse_dev_numbers("a:b"), None);
    }
}
