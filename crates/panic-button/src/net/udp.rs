// SPDX-FileCopyrightText: 2026 Open Health Technology Foundation contributors
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-PanicButton-app-store-exception

//! The UDP driver (`spec/03-protocol.md`, Transports and addressing).
//!
//! One socket bound to `0.0.0.0:PORT`, joined to the multicast group on
//! every interface. On Linux and macOS each datagram's destination address
//! (`IP_PKTINFO`) tells multicast, broadcast and unicast apart. Elsewhere
//! everything is reported as unicast: that is the safe mistake, because it
//! makes the core send more unicast, never less.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex};

use crate::core::io::{Input, InterfaceInfo, UdpDest, UdpKind};
use socket2::{Domain, Protocol, SockRef, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use crate::net::{NetConfig, NetError};

/// Largest datagram we accept (the protocol limit is 1200; allow slack so
/// oversize messages are counted as decode errors, not truncated).
const RECV_BUF: usize = 2048;

#[derive(Debug)]
pub struct UdpDriver {
    tx: mpsc::UnboundedSender<(UdpDest, Vec<u8>)>,
    shared: Arc<Shared>,
}

#[derive(Debug)]
struct Shared {
    sock: Arc<UdpSocket>,
    group: Ipv4Addr,
    port: u16,
    interfaces: Mutex<Vec<InterfaceInfo>>,
}

fn bind(cfg: &NetConfig) -> std::io::Result<std::net::UdpSocket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
    s.set_reuse_port(true)?;
    s.set_broadcast(true)?;
    s.set_multicast_loop_v4(true)?;
    s.set_multicast_ttl_v4(cfg.multicast_ttl)?;
    s.set_nonblocking(true)?;
    s.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, cfg.port).into())?;
    #[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
    {
        use nix::sys::socket::{setsockopt, sockopt};
        setsockopt(&s, sockopt::Ipv4PacketInfo, &true).map_err(std::io::Error::from)?;
    }
    Ok(s.into())
}

impl UdpDriver {
    pub fn start(
        cfg: &NetConfig,
        interfaces: &[InterfaceInfo],
        inputs: mpsc::Sender<Input>,
    ) -> Result<Self, NetError> {
        let sock = Arc::new(UdpSocket::from_std(bind(cfg)?)?);
        let shared = Arc::new(Shared {
            sock: sock.clone(),
            group: cfg.multicast_group,
            port: cfg.port,
            interfaces: Mutex::new(Vec::new()),
        });
        let driver = {
            let (tx, rx) = mpsc::unbounded_channel();
            tokio::spawn(send_loop(shared.clone(), rx, inputs.clone()));
            UdpDriver { tx, shared }
        };
        driver.set_interfaces(interfaces);
        tokio::spawn(recv_loop(driver.shared.clone(), inputs));
        Ok(driver)
    }

    /// Joins the group on new interfaces (and leaves old ones).
    pub fn set_interfaces(&self, now: &[InterfaceInfo]) {
        let mut cur = self
            .shared
            .interfaces
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for old in cur.iter() {
            if !now.contains(old) {
                let _ = self
                    .shared
                    .sock
                    .leave_multicast_v4(self.shared.group, old.addr);
            }
        }
        for new in now {
            if !cur.contains(new)
                && let Err(e) = self
                    .shared
                    .sock
                    .join_multicast_v4(self.shared.group, new.addr)
            {
                tracing::warn!("cannot join multicast on {}: {e}", new.addr);
            }
        }
        *cur = now.to_vec();
    }

    pub fn send(&self, dest: UdpDest, bytes: Vec<u8>) {
        let _ = self.tx.send((dest, bytes));
    }
}

/// macOS refuses local network sends with "no route to host" when the
/// Local Network permission is off.
const EHOSTUNREACH_MACOS: i32 = 65;

/// Whether a round of multicast or broadcast sends looks refused by the
/// macOS Local Network permission: there were sends, and every one failed
/// with "no route to host". `None` when there is nothing to judge by.
fn refused(results: &[std::io::Result<usize>]) -> Option<bool> {
    if results.is_empty() {
        return None;
    }
    Some(
        results
            .iter()
            .all(|r| matches!(r, Err(e) if e.raw_os_error() == Some(EHOSTUNREACH_MACOS))),
    )
}

async fn send_loop(
    sh: Arc<Shared>,
    mut rx: mpsc::UnboundedReceiver<(UdpDest, Vec<u8>)>,
    inputs: mpsc::Sender<Input>,
) {
    let mut blocked = false;
    while let Some((dest, bytes)) = rx.recv().await {
        let ifs = sh.interfaces.lock().map(|g| g.clone()).unwrap_or_default();
        let mut results = Vec::new();
        match dest {
            UdpDest::Unicast(a) => {
                if let Err(e) = sh.sock.send_to(&bytes, a).await {
                    tracing::debug!("udp send to {a}: {e}");
                }
            }
            UdpDest::Multicast => {
                // Out of every interface (sends are sequential in this task,
                // so setting the outgoing interface per send is safe).
                for i in &ifs {
                    if SockRef::from(&*sh.sock)
                        .set_multicast_if_v4(&i.addr)
                        .is_ok()
                    {
                        results.push(
                            sh.sock
                                .send_to(&bytes, SocketAddrV4::new(sh.group, sh.port))
                                .await,
                        );
                    }
                }
            }
            UdpDest::Broadcast => {
                results.push(
                    sh.sock
                        .send_to(&bytes, SocketAddrV4::new(Ipv4Addr::BROADCAST, sh.port))
                        .await,
                );
                for i in &ifs {
                    if let Some(b) = i.broadcast {
                        results.push(sh.sock.send_to(&bytes, SocketAddrV4::new(b, sh.port)).await);
                    }
                }
            }
        }
        if cfg!(target_os = "macos")
            && !ifs.is_empty()
            && let Some(now) = refused(&results)
            && now != blocked
        {
            blocked = now;
            tracing::warn!(blocked, "local network access");
            let _ = inputs.send(Input::LocalNetworkBlocked(blocked)).await;
        }
    }
}

fn classify(dst: Option<Ipv4Addr>, group: Ipv4Addr, ifs: &[InterfaceInfo]) -> UdpKind {
    match dst {
        Some(d) if d == group || d.is_multicast() => UdpKind::Multicast,
        Some(d) if d.is_broadcast() || ifs.iter().any(|i| i.broadcast == Some(d)) => {
            UdpKind::Broadcast
        }
        _ => UdpKind::Unicast,
    }
}

/// Datagrams accepted from one source address per second, before any
/// decryption (`spec/04-security.md`, denial of service). A device sends a
/// few a second at most, even while raising an alarm.
const PER_SOURCE_PER_SEC: u32 = 200;
/// Sources tracked per second. Beyond this (many spoofed addresses), new
/// sources are let through rather than dropped: alarms come first.
const SOURCES_MAX: usize = 4096;

/// Counts datagrams per source address in one-second windows.
struct SourceLimit {
    window: std::time::Instant,
    counts: std::collections::HashMap<Ipv4Addr, u32>,
}

impl SourceLimit {
    fn new(now: std::time::Instant) -> Self {
        SourceLimit {
            window: now,
            counts: std::collections::HashMap::new(),
        }
    }

    /// Whether to accept a datagram from `from` at `now`. Logs the first
    /// datagram dropped from a source in each window.
    fn accept(&mut self, from: Ipv4Addr, now: std::time::Instant) -> bool {
        if now.duration_since(self.window) >= std::time::Duration::from_secs(1) {
            self.window = now;
            self.counts.clear();
        }
        if self.counts.len() >= SOURCES_MAX && !self.counts.contains_key(&from) {
            return true;
        }
        let n = self.counts.entry(from).or_insert(0);
        *n = n.saturating_add(1);
        if *n == PER_SOURCE_PER_SEC + 1 {
            tracing::warn!(%from, "more than {PER_SOURCE_PER_SEC} datagrams a second; dropping the rest this second");
        }
        *n <= PER_SOURCE_PER_SEC
    }
}

async fn recv_loop(sh: Arc<Shared>, inputs: mpsc::Sender<Input>) {
    let mut buf = vec![0u8; RECV_BUF];
    let mut limit = SourceLimit::new(std::time::Instant::now());
    loop {
        let r = recv(&sh.sock, &mut buf).await;
        let (n, from, dst) = match r {
            Ok(x) => x,
            Err(e) => {
                tracing::debug!("udp recv: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                continue;
            }
        };
        let SocketAddr::V4(from) = from else { continue };
        if !limit.accept(*from.ip(), std::time::Instant::now()) {
            continue;
        }
        let kind = {
            let ifs = sh.interfaces.lock().map(|g| g.clone()).unwrap_or_default();
            classify(dst, sh.group, &ifs)
        };
        let input = Input::Datagram {
            kind,
            from,
            bytes: buf[..n].to_vec(),
        };
        if inputs.send(input).await.is_err() {
            return;
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
async fn recv(
    sock: &UdpSocket,
    buf: &mut [u8],
) -> std::io::Result<(usize, SocketAddr, Option<Ipv4Addr>)> {
    use std::io::IoSliceMut;
    use std::os::fd::AsRawFd;

    use nix::sys::socket::{ControlMessageOwned, MsgFlags, SockaddrIn, recvmsg};
    use tokio::io::Interest;
    loop {
        sock.readable().await?;
        let r = sock.try_io(Interest::READABLE, || {
            let mut iov = [IoSliceMut::new(buf)];
            let mut cmsg = nix::cmsg_space!(nix::libc::in_pktinfo);
            let msg = recvmsg::<SockaddrIn>(
                sock.as_raw_fd(),
                &mut iov,
                Some(&mut cmsg),
                MsgFlags::empty(),
            )
            .map_err(std::io::Error::from)?;
            let mut dst = None;
            if let Ok(cms) = msg.cmsgs() {
                for c in cms {
                    if let ControlMessageOwned::Ipv4PacketInfo(info) = c {
                        dst = Some(Ipv4Addr::from(u32::from_be(info.ipi_addr.s_addr)));
                    }
                }
            }
            let from = msg
                .address
                .map(|a| SocketAddr::V4(SocketAddrV4::new(a.ip(), a.port())))
                .ok_or_else(|| std::io::Error::other("no source address"))?;
            Ok((msg.bytes, from, dst))
        });
        match r {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            other => return other,
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
async fn recv(
    sock: &UdpSocket,
    buf: &mut [u8],
) -> std::io::Result<(usize, SocketAddr, Option<Ipv4Addr>)> {
    let (n, from) = sock.recv_from(buf).await?;
    Ok((n, from, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_most_200_datagrams_a_second_from_one_source() {
        let t0 = std::time::Instant::now();
        let mut l = SourceLimit::new(t0);
        let (a, b) = (Ipv4Addr::new(10, 0, 0, 1), Ipv4Addr::new(10, 0, 0, 2));
        let ms = |n| t0 + std::time::Duration::from_millis(n);
        assert_eq!((0..250).filter(|_| l.accept(a, ms(10))).count(), 200);
        assert!(l.accept(b, ms(20)), "another source is unaffected");
        assert!(l.accept(a, ms(1_000)), "the next second starts again");
        // Many sources: past the cap, new ones are let through.
        let mut l = SourceLimit::new(t0);
        for i in 0..SOURCES_MAX as u32 {
            assert!(l.accept(Ipv4Addr::from(i), t0));
        }
        let new = Ipv4Addr::new(192, 168, 1, 1);
        assert!((0..300).all(|_| l.accept(new, t0)));
    }

    #[test]
    fn local_network_refusal_is_every_send_failing_with_no_route() {
        let no_route = || Err(std::io::Error::from_raw_os_error(EHOSTUNREACH_MACOS));
        let other = || Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert_eq!(refused(&[]), None);
        assert_eq!(refused(&[no_route(), no_route()]), Some(true));
        assert_eq!(refused(&[no_route(), Ok(10)]), Some(false));
        assert_eq!(refused(&[other()]), Some(false));
    }

    #[test]
    fn classification() {
        let g = Ipv4Addr::new(239, 255, 71, 71);
        let ifs = [InterfaceInfo {
            addr: Ipv4Addr::new(10, 0, 0, 5),
            broadcast: Some(Ipv4Addr::new(10, 0, 0, 255)),
        }];
        assert_eq!(classify(Some(g), g, &ifs), UdpKind::Multicast);
        assert_eq!(
            classify(Some(Ipv4Addr::BROADCAST), g, &ifs),
            UdpKind::Broadcast
        );
        assert_eq!(
            classify(Some(Ipv4Addr::new(10, 0, 0, 255)), g, &ifs),
            UdpKind::Broadcast
        );
        assert_eq!(
            classify(Some(Ipv4Addr::new(10, 0, 0, 5)), g, &ifs),
            UdpKind::Unicast
        );
        assert_eq!(classify(None, g, &ifs), UdpKind::Unicast);
    }
}
