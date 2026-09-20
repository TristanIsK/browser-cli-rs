use std::{
    io,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::{Arc, Mutex, OnceLock, mpsc},
    thread,
};

use super::Deadline;
use crate::{Error, Result};

type Addresses = io::Result<Vec<SocketAddr>>;

struct Lookup {
    host: String,
    port: u16,
    deadline: Deadline,
    reply: mpsc::SyncSender<Addresses>,
}

struct Resolver(mpsc::SyncSender<Lookup>);

impl Resolver {
    fn new(resolve: impl Fn(&str, u16) -> Addresses + Send + Sync + 'static) -> io::Result<Self> {
        // ponytail: keep OS DNS (hosts/VPN support) with two workers and four
        // queued lookups. Stuck OS calls cannot be cancelled; capacity stays
        // bounded and saturation fails explicitly instead of spawning threads.
        let (sender, receiver) = mpsc::sync_channel::<Lookup>(4);
        let receiver = Arc::new(Mutex::new(receiver));
        let resolve = Arc::new(resolve);
        for _ in 0..2 {
            let receiver = receiver.clone();
            let resolve = resolve.clone();
            thread::Builder::new()
                .name("cdp-dns".into())
                .spawn(move || {
                    loop {
                        let Ok(job) = receiver.lock().unwrap().recv() else {
                            break;
                        };
                        if job.deadline.remaining("dns").is_ok() {
                            let result = resolve(&job.host, job.port);
                            // Only DNS happens here. A late result cannot open a socket.
                            let _ = job.reply.try_send(result);
                        }
                    }
                })?;
        }
        Ok(Self(sender))
    }

    fn lookup(
        &self,
        host: &str,
        port: u16,
        deadline: Deadline,
        stage: &str,
    ) -> Result<Vec<SocketAddr>> {
        deadline.remaining(stage)?;
        let (reply, receiver) = mpsc::sync_channel(1);
        self.0
            .try_send(Lookup {
                host: host.into(),
                port,
                deadline,
                reply,
            })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => Error::Cdp("DNS resolver busy; retry later".into()),
                mpsc::TrySendError::Disconnected(_) => {
                    Error::Cdp("DNS resolver unavailable".into())
                }
            })?;
        let result =
            receiver
                .recv_timeout(deadline.remaining(stage)?)
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => deadline.timeout(stage),
                    mpsc::RecvTimeoutError::Disconnected if deadline.remaining(stage).is_err() => {
                        deadline.timeout(stage)
                    }
                    mpsc::RecvTimeoutError::Disconnected => {
                        Error::Cdp("DNS resolver unavailable".into())
                    }
                })?;
        deadline.remaining(stage)?;
        result.map_err(|error| deadline.normalize(Error::Io(error), stage))
    }
}

pub(super) fn resolve(
    host: &str,
    port: u16,
    deadline: Deadline,
    stage: &str,
) -> Result<Vec<SocketAddr>> {
    deadline.remaining(stage)?;
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    static RESOLVER: OnceLock<io::Result<Resolver>> = OnceLock::new();
    let resolver = RESOLVER
        .get_or_init(|| {
            Resolver::new(|host, port| {
                (host, port)
                    .to_socket_addrs()
                    .map(|addresses| addresses.collect())
            })
        })
        .as_ref()
        .map_err(|_| Error::Cdp("cannot start DNS resolver".into()))?;
    resolver.lookup(host, port, deadline, stage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            Condvar,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    #[test]
    fn slow_dns_is_bounded_expired_work_is_skipped_and_pool_recovers() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let (started, running) = mpsc::channel();
        let resolver = Arc::new(
            Resolver::new({
                let gate = gate.clone();
                let calls = calls.clone();
                move |_, port| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    started.send(()).unwrap();
                    let (lock, ready) = &*gate;
                    let _guard = ready
                        .wait_while(lock.lock().unwrap(), |open| !*open)
                        .unwrap();
                    Ok(vec![SocketAddr::from(([127, 0, 0, 1], port))])
                }
            })
            .unwrap(),
        );
        let started_at = Instant::now();
        let mut callers = Vec::new();
        for _ in 0..2 {
            let resolver = resolver.clone();
            callers.push(thread::spawn(move || {
                resolver.lookup(
                    "slow.invalid",
                    80,
                    Deadline::new(Duration::from_millis(100)),
                    "proxy_dns",
                )
            }));
            running.recv_timeout(Duration::from_secs(2)).unwrap();
        }
        for caller in callers {
            assert!(
                matches!(caller.join().unwrap(), Err(Error::Timeout(message)) if message.contains("proxy_dns"))
            );
        }
        assert!(started_at.elapsed() < Duration::from_secs(2));
        // Both OS calls remain blocked. Four queued jobs fit, all later calls
        // fail immediately; no additional worker is started.
        let expired = Deadline::new(Duration::ZERO);
        for _ in 0..4 {
            let (reply, _) = mpsc::sync_channel(1);
            resolver
                .0
                .try_send(Lookup {
                    host: "expired.invalid".into(),
                    port: 80,
                    deadline: expired,
                    reply,
                })
                .unwrap();
        }
        for _ in 0..20 {
            assert!(
                matches!(resolver.lookup("extra.invalid", 80, Deadline::new(Duration::from_secs(1)), "dns"),
                Err(Error::Cdp(message)) if message.contains("busy"))
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        // Numeric IPs do not depend on the resolver pool.
        assert_eq!(
            resolve("::1", 80, Deadline::new(Duration::from_secs(1)), "dns").unwrap()[0].ip(),
            "::1".parse::<IpAddr>().unwrap()
        );
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        let recovery = Deadline::new(Duration::from_secs(2));
        loop {
            match resolver.lookup("recovered.invalid", 80, recovery, "dns") {
                Ok(_) => break,
                Err(Error::Cdp(message)) if message.contains("busy") => thread::yield_now(),
                other => panic!("pool did not recover: {other:?}"),
            }
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "expired queued jobs must not run"
        );
    }
}
