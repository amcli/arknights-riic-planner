//! Limits on how fast one client may call, and on how much heavy work one
//! account may have going at once.
//!
//! Every client ([`crate::client`]) has a budget of requests that refills at
//! a steady rate up to a burst ([`Rate`]). All routes draw on one budget;
//! signing up and signing in also draw on a much smaller second one. A
//! request over budget is answered `429` with `Retry-After` before any other
//! work is done. IPv6 clients are counted by their /64, the smallest block a
//! network is given, so that hopping between the addresses of one network
//! (privacy addresses, say) does not buy a fresh budget; a client holding a
//! larger block still gets one budget per /64, as a client holding many IPv4
//! addresses gets one per address.
//!
//! Each account may also have only so many simulations computing at once
//! ([`InFlight`]). A slot is held until its computation ends, even if the
//! request that started it has gone, since the computation carries on.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use governor::clock::Clock;
use governor::{DefaultKeyedRateLimiter, Quota};
use ipnet::IpNet;
use uuid::Uuid;

use crate::AppState;
use crate::client::client_ip;
use crate::error::{ApiError, whole_seconds};

/// A budget of requests: `burst` at once, refilling at `per_minute`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    /// Requests the budget regains per minute.
    pub per_minute: NonZeroU32,
    /// Most requests the budget holds, and so the most in a quick burst.
    pub burst: NonZeroU32,
}

/// A per-client budget of requests.
#[derive(Debug)]
pub struct RateLimit {
    /// What is counted, for the message: "requests", "sign-ups and sign-ins".
    what: &'static str,
    limiter: DefaultKeyedRateLimiter<IpAddr>,
    /// Proxies trusted to name the client.
    trusted: Arc<[IpNet]>,
}

impl RateLimit {
    /// A budget of `rate` per client, counting `what`.
    pub fn new(what: &'static str, rate: Rate, trusted: Arc<[IpNet]>) -> Self {
        let quota = Quota::per_minute(rate.per_minute).allow_burst(rate.burst);
        RateLimit {
            what,
            limiter: DefaultKeyedRateLimiter::keyed(quota),
            trusted,
        }
    }

    /// Takes one request from the client's budget, or answers how long the
    /// client must wait. A request with no client (made in-process) is not
    /// counted.
    pub fn check<B>(&self, req: &axum::http::Request<B>) -> Result<(), ApiError> {
        let Some(ip) = client_ip(req, &self.trusted) else {
            return Ok(());
        };
        self.limiter.check_key(&rate_key(ip)).map_err(|not_until| {
            let wait = not_until.wait_time_from(self.limiter.clock().now());
            ApiError::too_many_requests(format!(
                "too many {} from your address; try again in {} s",
                self.what,
                whole_seconds(wait)
            ))
            .retry_after(wait)
        })
    }

    /// Drops clients whose budget has refilled, so that memory follows the
    /// clients seen lately rather than all clients ever.
    pub fn forget_full(&self) {
        self.limiter.retain_recent();
        self.limiter.shrink_to_fit();
    }

    /// Clients being tracked.
    pub fn tracked(&self) -> usize {
        self.limiter.len()
    }
}

/// The key a client is counted under: its IPv4 address, or its IPv6 /64.
fn rate_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from(u128::from(v6) & !u128::from(u64::MAX))),
        v4 => v4,
    }
}

/// Middleware: answers `429` once the client has used up `limit`.
pub async fn limit(State(limit): State<Arc<RateLimit>>, req: Request, next: Next) -> Response {
    match limit.check(&req) {
        Ok(()) => next.run(req).await,
        Err(e) => e.into_response(),
    }
}

/// Heavy work in flight per account, up to a cap.
#[derive(Debug)]
pub struct InFlight {
    /// What is counted, for the message: "simulations".
    what: &'static str,
    max: NonZeroUsize,
    counts: Mutex<HashMap<Uuid, usize>>,
}

/// A held slot; dropping it frees the slot.
#[derive(Debug)]
pub struct Slot {
    flight: Arc<InFlight>,
    owner: Uuid,
}

impl InFlight {
    /// At most `max` of `what` per account at once.
    pub fn new(what: &'static str, max: NonZeroUsize) -> Self {
        InFlight {
            what,
            max,
            counts: Mutex::new(HashMap::new()),
        }
    }

    /// A slot for `owner`, or `429` when all of theirs are taken.
    pub fn take(self: &Arc<Self>, owner: Uuid) -> Result<Slot, ApiError> {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        let count = counts.entry(owner).or_default();
        if *count >= self.max.get() {
            return Err(ApiError::too_many_requests(format!(
                "you already have {} {} running; try again when one has finished",
                self.max, self.what
            )));
        }
        *count += 1;
        Ok(Slot {
            flight: self.clone(),
            owner,
        })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut counts = self
            .flight
            .counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = counts.get_mut(&self.owner) {
            *count -= 1;
            if *count == 0 {
                counts.remove(&self.owner);
            }
        }
    }
}

/// How often [`sweep`] tidies the limits.
const SWEEP_EVERY: Duration = Duration::from_secs(60);

/// Every minute, forgets clients whose budgets have refilled. Runs until
/// the process exits.
pub async fn sweep(state: AppState) {
    let mut every = tokio::time::interval(SWEEP_EVERY);
    loop {
        every.tick().await;
        state.requests.forget_full();
        state.sign_ins.forget_full();
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::extract::ConnectInfo;
    use axum::http::{Request, StatusCode, header};

    use super::*;

    fn from(addr: &str) -> Request<()> {
        let mut req = Request::new(());
        let addr: SocketAddr = addr.parse().unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        req
    }

    fn rate(per_minute: u32, burst: u32) -> Rate {
        Rate {
            per_minute: NonZeroU32::new(per_minute).unwrap(),
            burst: NonZeroU32::new(burst).unwrap(),
        }
    }

    #[test]
    fn a_client_gets_its_burst_then_waits() {
        let limit = RateLimit::new("requests", rate(1, 2), Arc::from([]));
        assert!(limit.check(&from("198.51.100.1:5000")).is_ok());
        assert!(limit.check(&from("198.51.100.1:5001")).is_ok());
        let e = limit.check(&from("198.51.100.1:5002")).unwrap_err();
        assert_eq!(e.status, StatusCode::TOO_MANY_REQUESTS);
        let wait = e.retry_after.unwrap();
        assert!(
            wait > Duration::from_secs(58) && wait <= Duration::from_secs(60),
            "{wait:?}"
        );
        let resp = e.into_response();
        assert_eq!(resp.headers()[header::RETRY_AFTER], "60");
        // Someone else has their own budget; in-process requests none.
        assert!(limit.check(&from("198.51.100.2:5000")).is_ok());
        for _ in 0..5 {
            assert!(limit.check(&Request::new(())).is_ok());
        }
        assert_eq!(limit.tracked(), 2);
    }

    #[test]
    fn clients_whose_budget_has_refilled_are_forgotten() {
        // A token every millisecond: the budget refills almost at once.
        let limit = RateLimit::new("requests", rate(60_000, 1), Arc::from([]));
        assert!(limit.check(&from("198.51.100.1:5000")).is_ok());
        assert_eq!(limit.tracked(), 1);
        std::thread::sleep(Duration::from_millis(20));
        limit.forget_full();
        assert_eq!(limit.tracked(), 0);
        // A client still waiting is kept.
        let slow = RateLimit::new("requests", rate(1, 1), Arc::from([]));
        assert!(slow.check(&from("198.51.100.1:5000")).is_ok());
        slow.forget_full();
        assert_eq!(slow.tracked(), 1);
    }

    #[test]
    fn ipv6_clients_are_counted_by_their_64() {
        let limit = RateLimit::new("requests", rate(1, 1), Arc::from([]));
        assert!(limit.check(&from("[2001:db8:1:2::1]:5000")).is_ok());
        assert!(limit.check(&from("[2001:db8:1:2::ffff]:5000")).is_err());
        assert!(limit.check(&from("[2001:db8:1:3::1]:5000")).is_ok());
        // An IPv4 address seen as IPv6 is the IPv4 client.
        assert!(limit.check(&from("192.0.2.7:5000")).is_ok());
        assert!(limit.check(&from("[::ffff:192.0.2.7]:5000")).is_err());
    }

    #[test]
    fn behind_a_trusted_proxy_each_client_has_its_own_budget() {
        let trusted: Arc<[IpNet]> = Arc::from([crate::client::parse_trusted("10.0.0.1").unwrap()]);
        let limit = RateLimit::new("requests", rate(1, 1), trusted);
        let via_proxy = |client: &str| {
            let mut req = from("10.0.0.1:5000");
            req.headers_mut()
                .insert("x-forwarded-for", client.parse().unwrap());
            req
        };
        assert!(limit.check(&via_proxy("203.0.113.1")).is_ok());
        assert!(limit.check(&via_proxy("203.0.113.2")).is_ok());
        assert!(limit.check(&via_proxy("203.0.113.1")).is_err());
        // Claiming another address in front of the proxy's entry buys nothing.
        assert!(limit.check(&via_proxy("192.0.2.99, 203.0.113.1")).is_err());
        // A dual-stack listener reports the IPv4 proxy as ::ffff:10.0.0.1.
        let mut req = from("[::ffff:10.0.0.1]:5000");
        req.headers_mut()
            .insert("x-forwarded-for", "203.0.113.3".parse().unwrap());
        assert!(limit.check(&req).is_ok());
        assert!(limit.check(&via_proxy("203.0.113.3")).is_err());
    }

    #[test]
    fn slots_are_per_account_and_freed_on_drop() {
        let flight = Arc::new(InFlight::new("simulations", NonZeroUsize::new(2).unwrap()));
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let first = flight.take(a).unwrap();
        let _second = flight.take(a).unwrap();
        let e = flight.take(a).unwrap_err();
        assert_eq!(e.status, StatusCode::TOO_MANY_REQUESTS);
        assert!(e.retry_after.is_none());
        assert!(flight.take(b).is_ok());
        drop(first);
        assert!(flight.take(a).is_ok());
    }
}
