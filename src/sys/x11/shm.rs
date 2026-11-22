//! A pixel buffer shared with the X server through MIT-SHM, so a frame is not copied through the X socket.
//!
//! The memory is an anonymous `memfd` mapped into this process and attached to the server with `ShmAttachFd`
//! (MIT-SHM 1.2). That only works when the server runs on the same machine, which `create` finds out by itself.
//! Platforms without `memfd_create` get no shared buffer and the overlay falls back to `PutImage`.

use super::conn::Conn;
use std::sync::Arc;

#[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
mod imp {
    use super::*;
    use rustix::fs::{ftruncate, memfd_create, MemfdFlags};
    use rustix::mm::{mmap, munmap, MapFlags, ProtFlags};
    use x11rb::connection::Connection;
    use x11rb::protocol::shm::{ConnectionExt as _, Seg};

    pub struct ShmBuffer {
        c: Arc<Conn>,
        ptr: *mut u8,
        len: usize,
        seg: Seg,
    }

    impl ShmBuffer {
        pub fn create(c: &Arc<Conn>, len: usize) -> Result<Self, String> {
            let version = c
                .conn
                .shm_query_version()
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(|e| format!("the X server has no MIT-SHM extension: {e}"))?;
            if (version.major_version, version.minor_version) < (1, 2) {
                return Err("MIT-SHM older than 1.2 cannot attach file descriptors".into());
            }

            let fd =
                memfd_create("snowfall-overlay", MemfdFlags::CLOEXEC).map_err(|e| e.to_string())?;
            ftruncate(&fd, len as u64).map_err(|e| e.to_string())?;
            // SAFETY: a fresh shared mapping of a file we own, not tied to any existing Rust object.
            let ptr = unsafe {
                mmap(
                    std::ptr::null_mut(),
                    len,
                    ProtFlags::READ | ProtFlags::WRITE,
                    MapFlags::SHARED,
                    &fd,
                    0,
                )
            }
            .map_err(|e| e.to_string())? as *mut u8;

            // The mapping outlives the descriptor, which is handed over to the server.
            let seg: Seg = c.conn.generate_id().map_err(|e| e.to_string())?;
            let attached = c
                .conn
                .shm_attach_fd(seg, fd, false)
                .map_err(|e| e.to_string())
                .and_then(|cookie| cookie.check().map_err(|e| e.to_string()));
            if let Err(e) = attached {
                // SAFETY: `ptr`/`len` are the mapping created above and nothing else uses it yet.
                let _ = unsafe { munmap(ptr.cast(), len) };
                return Err(format!("the X server refused the shared segment: {e}"));
            }

            Ok(Self {
                c: c.clone(),
                ptr,
                len,
                seg,
            })
        }

        pub fn seg(&self) -> Seg {
            self.seg
        }

        /// The pixels. The X server reads the same memory while a `ShmPutImage` is pending,
        /// so callers must wait for its completion event before writing again.
        pub fn as_mut_slice(&mut self) -> &mut [u8] {
            // SAFETY: `ptr` points to a live mapping of `len` bytes owned by `self`, and `&mut self` makes this
            // the only Rust reference to it.
            unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
        }
    }

    impl Drop for ShmBuffer {
        fn drop(&mut self) {
            let _ = self.c.conn.shm_detach(self.seg);
            let _ = self.c.conn.flush();
            // SAFETY: the mapping created in `create`, unmapped exactly once.
            let _ = unsafe { munmap(self.ptr.cast(), self.len) };
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "freebsd")))]
mod imp {
    use super::*;

    /// Never constructed: there is no shared memory support on this platform.
    pub enum ShmBuffer {}

    impl ShmBuffer {
        pub fn create(_c: &Arc<Conn>, _len: usize) -> Result<Self, String> {
            Err("shared memory frames need memfd_create".into())
        }

        pub fn seg(&self) -> u32 {
            match *self {}
        }

        pub fn as_mut_slice(&mut self) -> &mut [u8] {
            match *self {}
        }
    }
}

pub use imp::ShmBuffer;
