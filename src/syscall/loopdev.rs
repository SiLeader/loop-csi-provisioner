use crate::syscall::Syscall;
use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::{Getter, NoArg, Setter, ioctl};
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};

impl Syscall {
    pub async fn find_loop(&self, file: impl AsRef<Path>) -> std::io::Result<PathBuf> {
        let file = file.as_ref().to_path_buf();

        Self::spawn(move || {
            let lc = LoopControl::open()?;
            let ld = lc.next_free()?;
            ld.attach_file(file.as_ref())?;
            Ok(ld.into_path())
        })
        .await
    }

    pub async fn apply_loop_device_capacity(
        &self,
        loop_device: impl AsRef<Path>,
    ) -> std::io::Result<()> {
        let ld = loop_device.as_ref().to_path_buf();
        Self::spawn(move || {
            let ld = LoopDevice::open(ld)?;
            ld.apply_capacity()
        })
        .await
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

struct LoopControl {
    file: OwnedFd,
}

struct LoopDevice {
    path: PathBuf,
    file: OwnedFd,
}

const LOOP_CTL_GET_FREE: rustix::ioctl::Opcode = linux_raw_sys::loop_device::LOOP_CTL_GET_FREE;
const LOOP_CONFIGURE: rustix::ioctl::Opcode = linux_raw_sys::loop_device::LOOP_CONFIGURE;
const LOOP_CLR_FD: rustix::ioctl::Opcode = linux_raw_sys::loop_device::LOOP_CLR_FD;
const LOOP_SET_CAPACITY: rustix::ioctl::Opcode = linux_raw_sys::loop_device::LOOP_SET_CAPACITY;

const LO_FLAGS_READ_ONLY: u32 = linux_raw_sys::loop_device::LO_FLAGS_READ_ONLY as u32;
const LO_FLAGS_AUTOCLEAR: u32 = linux_raw_sys::loop_device::LO_FLAGS_AUTOCLEAR as u32;
const LO_FLAGS_PARTSCAN: u32 = linux_raw_sys::loop_device::LO_FLAGS_PARTSCAN as u32;

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
struct LoopConfig {
    fd: u32,
    block_size: u32,
    info: LoopInfo64,
    reserved: [u64; 8],
}

impl LoopControl {
    fn open() -> std::io::Result<Self> {
        let file = open(
            "/dev/loop-device",
            OFlags::RDWR | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        Ok(Self { file })
    }

    fn next_free(&self) -> std::io::Result<LoopDevice> {
        let num = unsafe {
            let ctl = Getter::<LOOP_CTL_GET_FREE, i32>::new();
            ioctl(&self.file, ctl)?
        };
        if num < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            let path = PathBuf::from(format!("/dev/loop{}", num));
            LoopDevice::open(path)
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

    fn attach_file(&self, file: &Path) -> std::io::Result<()> {
        let image_fd = open(file, OFlags::RDWR | OFlags::CLOEXEC, Mode::empty())?;

        let config = LoopConfig {
            fd: image_fd.as_raw_fd() as u32,
            block_size: 0,
            info: LoopInfo64 {
                lo_flags: LO_FLAGS_AUTOCLEAR,
                ..Default::default()
            },
            ..Default::default()
        };

        unsafe {
            let ctl = Setter::<LOOP_CONFIGURE, &LoopConfig>::new(&config);
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

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            fd: 0,
            block_size: 0,
            info: Default::default(),
            reserved: [0u64; 8],
        }
    }
}
