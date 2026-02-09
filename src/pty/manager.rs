use std::collections::HashMap;
use std::os::unix::io::{IntoRawFd, RawFd};

use anyhow::Result;
use tokio::sync::broadcast;

/// Manages PTY master file descriptors and bridges them to clients.
pub struct PtyManager {
    ptys: HashMap<u32, PtyHandle>,
    next_id: u32,
}

pub struct PtyHandle {
    pub id: u32,
    pub master_fd: RawFd,
    pub output_tx: broadcast::Sender<bytes::Bytes>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self {
            ptys: HashMap::new(),
            next_id: 0,
        }
    }

    /// Allocate a new PTY. Returns (pty_id, master_fd, slave_fd).
    pub fn allocate(&mut self) -> Result<(u32, RawFd, RawFd)> {
        let pty = nix::pty::openpty(None, None)?;
        let id = self.next_id;
        self.next_id += 1;

        let master_fd = pty.master.into_raw_fd();
        let slave_fd = pty.slave.into_raw_fd();

        let (output_tx, _) = broadcast::channel(256);

        self.ptys.insert(
            id,
            PtyHandle {
                id,
                master_fd,
                output_tx,
            },
        );

        Ok((id, master_fd, slave_fd))
    }

    /// Get a broadcast receiver for PTY output
    pub fn subscribe(&self, id: u32) -> Option<broadcast::Receiver<bytes::Bytes>> {
        self.ptys.get(&id).map(|h| h.output_tx.subscribe())
    }

    /// Remove a PTY
    pub fn remove(&mut self, id: u32) -> Option<PtyHandle> {
        self.ptys.remove(&id)
    }
}
