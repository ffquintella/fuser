use std::{
    fs::File,
    io,
    os::{
        fd::{AsFd, BorrowedFd},
        unix::prelude::AsRawFd,
    },
    sync::Arc,
};

use libc::{c_int, c_void, size_t};

#[cfg(feature = "abi-7-40")]
use crate::passthrough::BackingId;
use crate::reply::ReplySender;

/// A raw communication channel to the FUSE kernel driver
#[derive(Debug)]
pub struct Channel(
    Arc<File>,
    /// Bytes read past the end of the previous request (FUSE-T socket framing).
    #[cfg(target_os = "macos")]
    std::sync::Mutex<Vec<u8>>,
);

impl AsFd for Channel {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl Channel {
    /// Create a new communication channel to the kernel driver by mounting the
    /// given path. The kernel driver will delegate filesystem operations of
    /// the given path to the channel.
    pub(crate) fn new(device: Arc<File>) -> Self {
        #[cfg(target_os = "macos")]
        {
            Self(device, std::sync::Mutex::new(Vec::new()))
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self(device)
        }
    }

    /// Receives one request, up to the capacity of the given buffer (can block).
    pub fn receive(&self, buffer: &mut [u8]) -> io::Result<usize> {
        #[cfg(target_os = "macos")]
        {
            self.receive_framed(buffer)
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.read_once(buffer)
        }
    }

    fn read_once(&self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            let rc = unsafe {
                libc::read(
                    self.0.as_raw_fd(),
                    buffer.as_mut_ptr() as *mut c_void,
                    buffer.len() as size_t,
                )
            };
            if rc >= 0 {
                return Ok(rc as usize);
            }
            let err = io::Error::last_os_error();
            if err.kind() != io::ErrorKind::Interrupted {
                return Err(err);
            }
        }
    }

    /// FUSE-T hands requests over a socket, not `/dev/fuse`: a WRITE arrives as a header
    /// followed by a separate payload, so a single `read` can return a partial request (and a
    /// stream could return more than one). Assemble exactly one request using the length at the
    /// start of the header, and keep any excess for the next call.
    #[cfg(target_os = "macos")]
    fn receive_framed(&self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut pending = self.1.lock().unwrap_or_else(|e| e.into_inner());
        let mut filled = if pending.is_empty() {
            self.read_once(buffer)?
        } else {
            let n = pending.len().min(buffer.len());
            buffer[..n].copy_from_slice(&pending[..n]);
            pending.drain(..n);
            n
        };
        if filled == 0 {
            return Ok(0);
        }
        loop {
            // Need the 4-byte length first, then the rest of the message.
            let want = if filled < 4 {
                4
            } else {
                u32::from_ne_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize
            };
            if want > buffer.len() {
                // Not a plausible request; let the parser report it.
                return Ok(filled);
            }
            if filled >= want {
                if filled > want {
                    let mut rest = buffer[want..filled].to_vec();
                    rest.extend_from_slice(&pending);
                    *pending = rest;
                }
                return Ok(want);
            }
            let n = self.read_once(&mut buffer[filled..want])?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "FUSE request truncated by the peer",
                ));
            }
            filled += n;
        }
    }

    /// Returns a sender object for this channel. The sender object can be
    /// used to send to the channel. Multiple sender objects can be used
    /// and they can safely be sent to other threads.
    pub fn sender(&self) -> ChannelSender {
        // Since write/writev syscalls are threadsafe, we can simply create
        // a sender by using the same file and use it in other threads.
        ChannelSender(self.0.clone())
    }
}

#[derive(Clone, Debug)]
pub struct ChannelSender(Arc<File>);

impl ReplySender for ChannelSender {
    fn send(&self, bufs: &[io::IoSlice<'_>]) -> io::Result<()> {
        let rc = unsafe {
            libc::writev(
                self.0.as_raw_fd(),
                bufs.as_ptr() as *const libc::iovec,
                bufs.len() as c_int,
            )
        };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            debug_assert_eq!(bufs.iter().map(|b| b.len()).sum::<usize>(), rc as usize);
            Ok(())
        }
    }

    #[cfg(feature = "abi-7-40")]
    fn open_backing(&self, fd: BorrowedFd<'_>) -> std::io::Result<BackingId> {
        BackingId::create(&self.0, fd)
    }
}
