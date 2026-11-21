// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! The TCP driver: frames are a `u32` big-endian length then one envelope.
//! Outbound connections open on demand (1 s connect timeout), stay open for
//! 60 s idle, and are limited to 64 at once (`spec/03-protocol.md`).

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::core::io::Input;
use crate::proto::limits::TCP_FRAME_MAX;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::net::NetError;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
pub const MAX_OUTBOUND: usize = 64;
/// Frames queued per connection before new ones are dropped.
const QUEUE: usize = 256;

type Pool = Arc<Mutex<HashMap<SocketAddrV4, mpsc::Sender<Vec<u8>>>>>;

#[derive(Debug)]
pub struct TcpDriver {
    port: u16,
    pool: Pool,
}

impl TcpDriver {
    /// Listens on `port`, or an ephemeral port if it is busy.
    pub async fn start(port: u16, inputs: mpsc::Sender<Input>) -> Result<Self, NetError> {
        let listener = match TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)).await
        {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("TCP port {port} unavailable ({e}); using an ephemeral port");
                TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).await?
            }
        };
        let port = listener.local_addr()?.port();
        tokio::spawn(accept_loop(listener, inputs));
        Ok(TcpDriver {
            port,
            pool: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn send(&self, to: SocketAddrV4, frame: Vec<u8>) {
        if frame.len() > TCP_FRAME_MAX {
            return;
        }
        let mut pool = self.pool.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tx) = pool.get(&to) {
            match tx.try_send(frame) {
                Ok(()) => return,
                Err(mpsc::error::TrySendError::Full(_)) => return,
                Err(mpsc::error::TrySendError::Closed(f)) => {
                    pool.remove(&to);
                    return self.open(&mut pool, to, f);
                }
            }
        }
        self.open(&mut pool, to, frame);
    }

    fn open(
        &self,
        pool: &mut HashMap<SocketAddrV4, mpsc::Sender<Vec<u8>>>,
        to: SocketAddrV4,
        first: Vec<u8>,
    ) {
        pool.retain(|_, tx| !tx.is_closed());
        if pool.len() >= MAX_OUTBOUND {
            tracing::debug!("TCP pool full; dropping frame to {to}");
            return;
        }
        let (tx, rx) = mpsc::channel(QUEUE);
        let _ = tx.try_send(first);
        pool.insert(to, tx);
        let p = self.pool.clone();
        tokio::spawn(async move {
            if let Err(e) = connection(to, rx).await {
                tracing::debug!("TCP to {to}: {e}");
            }
            p.lock().unwrap_or_else(|e| e.into_inner()).remove(&to);
        });
    }
}

async fn connection(to: SocketAddrV4, mut rx: mpsc::Receiver<Vec<u8>>) -> std::io::Result<()> {
    let mut s = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(to))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "connect timeout"))??;
    s.set_nodelay(true)?;
    loop {
        let frame = match tokio::time::timeout(IDLE_TIMEOUT, rx.recv()).await {
            Ok(Some(f)) => f,
            Ok(None) | Err(_) => return Ok(()),
        };
        let len = u32::try_from(frame.len()).map_err(|_| std::io::Error::other("frame size"))?;
        s.write_all(&len.to_be_bytes()).await?;
        s.write_all(&frame).await?;
    }
}

async fn accept_loop(listener: TcpListener, inputs: mpsc::Sender<Input>) {
    loop {
        match listener.accept().await {
            Ok((s, SocketAddr::V4(from))) => {
                let inputs = inputs.clone();
                tokio::spawn(async move {
                    if let Err(e) = read_frames(s, from, inputs).await {
                        tracing::debug!("TCP from {from}: {e}");
                    }
                });
            }
            Ok(_) => {}
            Err(e) => {
                tracing::debug!("accept: {e}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

async fn read_frames(
    mut s: TcpStream,
    from: SocketAddrV4,
    inputs: mpsc::Sender<Input>,
) -> std::io::Result<()> {
    loop {
        let mut len = [0u8; 4];
        match tokio::time::timeout(IDLE_TIMEOUT * 2, s.read_exact(&mut len)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Ok(Err(e)) => return Err(e),
            Err(_) => return Ok(()),
        }
        let n = u32::from_be_bytes(len) as usize;
        if n > TCP_FRAME_MAX {
            return Err(std::io::Error::other("oversized frame"));
        }
        let mut buf = vec![0u8; n];
        s.read_exact(&mut buf).await?;
        if inputs
            .send(Input::StreamFrame { from, bytes: buf })
            .await
            .is_err()
        {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `03-protocol.md`, Size limits: a frame over 256 KiB is refused and
    /// the connection closed, before reading its body.
    #[tokio::test]
    async fn oversized_frames_close_the_connection() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (tx, mut rx) = mpsc::channel(8);
        let a = TcpDriver::start(0, tx).await.expect("listen");
        let mut s = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, a.port()))
            .await
            .expect("connect");
        s.write_all(&((TCP_FRAME_MAX + 1) as u32).to_be_bytes())
            .await
            .expect("write");
        let mut buf = [0u8; 1];
        let n = tokio::time::timeout(Duration::from_secs(5), s.read(&mut buf))
            .await
            .expect("closed in time")
            .unwrap_or(0);
        assert_eq!(n, 0, "the connection is closed");
        assert!(rx.try_recv().is_err(), "nothing reached the core");
    }

    #[tokio::test]
    async fn frames_round_trip_over_loopback() {
        let (tx, mut rx) = mpsc::channel(8);
        let a = TcpDriver::start(0, tx).await.expect("listen");
        let (tx2, _rx2) = mpsc::channel(8);
        let b = TcpDriver::start(0, tx2).await.expect("listen");
        b.send(
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, a.port()),
            vec![1, 2, 3],
        );
        b.send(SocketAddrV4::new(Ipv4Addr::LOCALHOST, a.port()), vec![4]);
        for want in [vec![1, 2, 3], vec![4]] {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                Ok(Some(Input::StreamFrame { bytes, .. })) => assert_eq!(bytes, want),
                other => panic!("unexpected {other:?}"),
            }
        }
    }
}
