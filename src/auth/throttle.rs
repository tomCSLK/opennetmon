//! Slowing down password guessing at the login.
//!
//! Failed logins are counted per user name and per client address within a sliding window. Past a
//! threshold, further attempts are *refused* (without checking the password) for a time that doubles
//! with every additional failure. Refusing instead of locking an account matters: an attacker must
//! not be able to keep the real administrator out.
//!
//! - **Per user name:** gentle (5 failures, then 5 s doubling up to 60 s). Even a botnet that rotates
//!   addresses can only try about one password per minute and account, while the legitimate user is
//!   delayed by at most a minute.
//! - **Per address:** strict (10 failures, then 30 s doubling up to 15 min), so one machine cannot
//!   walk through many accounts. IPv6 addresses are grouped by their /64 prefix.
//!
//! State lives in memory (a restart forgets it, which only helps an attacker once) and is bounded.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::Mutex;

const WINDOW_MS: i64 = 15 * 60 * 1000;
const MAX_ENTRIES: usize = 10_000;
const MAX_NAME_CHARS: usize = 64;

#[derive(Debug, Clone, Copy)]
struct Policy {
    threshold: u32,
    base_ms: i64,
    cap_ms: i64,
}

const USER_POLICY: Policy = Policy {
    threshold: 5,
    base_ms: 5_000,
    cap_ms: 60_000,
};
const ADDRESS_POLICY: Policy = Policy {
    threshold: 10,
    base_ms: 30_000,
    cap_ms: 900_000,
};

impl Policy {
    /// How long attempts are refused after the latest of `failures` failures.
    fn wait_ms(self, failures: u32) -> i64 {
        if failures < self.threshold {
            return 0;
        }
        let doublings = (failures - self.threshold).min(20);
        self.base_ms.saturating_mul(1 << doublings).min(self.cap_ms)
    }
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    failures: u32,
    last_ms: i64,
}

#[derive(Default)]
struct State {
    users: HashMap<String, Entry>,
    addresses: HashMap<IpAddr, Entry>,
}

#[derive(Default)]
pub struct LoginThrottle {
    state: Mutex<State>,
}

fn user_key(username: &str) -> String {
    username
        .trim()
        .chars()
        .take(MAX_NAME_CHARS)
        .flat_map(char::to_lowercase)
        .collect()
}

/// Groups IPv6 addresses by /64 so a single host cannot rotate through its whole prefix.
fn address_key(ip: Option<IpAddr>) -> IpAddr {
    match ip {
        Some(IpAddr::V6(v6)) => {
            let s = v6.segments();
            IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
        }
        Some(v4) => v4,
        None => IpAddr::from([0, 0, 0, 0]),
    }
}

fn purge<K: std::hash::Hash + Eq + Clone>(map: &mut HashMap<K, Entry>, now_ms: i64) {
    map.retain(|_, e| now_ms.saturating_sub(e.last_ms) <= WINDOW_MS);
    if map.len() >= MAX_ENTRIES {
        // Under an address flood keep only the newest half.
        let mut times: Vec<i64> = map.values().map(|e| e.last_ms).collect();
        times.sort_unstable();
        let cutoff = times[times.len() / 2];
        map.retain(|_, e| e.last_ms > cutoff);
    }
}

fn bump<K: std::hash::Hash + Eq + Clone>(map: &mut HashMap<K, Entry>, key: K, now_ms: i64) {
    purge(map, now_ms);
    let entry = map.entry(key).or_insert(Entry {
        failures: 0,
        last_ms: now_ms,
    });
    entry.failures = entry.failures.saturating_add(1);
    entry.last_ms = now_ms;
}

fn remaining<K: std::hash::Hash + Eq>(
    map: &HashMap<K, Entry>,
    key: &K,
    policy: Policy,
    now_ms: i64,
) -> i64 {
    map.get(key).map_or(0, |e| {
        if now_ms.saturating_sub(e.last_ms) > WINDOW_MS {
            return 0;
        }
        (e.last_ms + policy.wait_ms(e.failures) - now_ms).max(0)
    })
}

impl LoginThrottle {
    /// `Err(milliseconds to wait)` if an attempt for this user name from this address must be refused now.
    pub fn check(&self, now_ms: i64, username: &str, ip: Option<IpAddr>) -> Result<(), i64> {
        let Ok(state) = self.state.lock() else {
            return Ok(()); // a poisoned lock must never lock everybody out
        };
        let wait = remaining(&state.users, &user_key(username), USER_POLICY, now_ms).max(
            remaining(&state.addresses, &address_key(ip), ADDRESS_POLICY, now_ms),
        );
        if wait > 0 { Err(wait) } else { Ok(()) }
    }

    pub fn record_failure(&self, now_ms: i64, username: &str, ip: Option<IpAddr>) {
        if let Ok(mut state) = self.state.lock() {
            bump(&mut state.users, user_key(username), now_ms);
            bump(&mut state.addresses, address_key(ip), now_ms);
        }
    }

    /// A successful login clears the counters of that user name and address.
    pub fn record_success(&self, username: &str, ip: Option<IpAddr>) {
        if let Ok(mut state) = self.state.lock() {
            state.users.remove(&user_key(username));
            state.addresses.remove(&address_key(ip));
        }
    }
}

/// Which request header carries the real client address when a reverse proxy is in front.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClientIpHeader {
    /// `X-Forwarded-For`; the *last* entry is used (the one the trusted proxy appended).
    XForwardedFor,
    /// `X-Real-IP`.
    XRealIp,
}

/// The client address: from the configured proxy header if present and valid, else the TCP peer.
pub fn client_ip(
    peer: Option<IpAddr>,
    headers: &axum::http::HeaderMap,
    header: Option<ClientIpHeader>,
) -> Option<IpAddr> {
    let from_header = header.and_then(|h| {
        let (name, last_entry) = match h {
            ClientIpHeader::XForwardedFor => ("x-forwarded-for", true),
            ClientIpHeader::XRealIp => ("x-real-ip", false),
        };
        let value = headers.get(name)?.to_str().ok()?;
        let entry = if last_entry {
            value.rsplit(',').next()?
        } else {
            value
        };
        entry.trim().parse::<IpAddr>().ok()
    });
    from_header.or(peer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    const T0: i64 = 1_000_000;

    fn ip(s: &str) -> Option<IpAddr> {
        Some(s.parse().unwrap())
    }

    fn fail(t: &LoginThrottle, now: i64, user: &str, addr: &str) {
        t.record_failure(now, user, ip(addr));
    }

    #[test]
    fn the_wait_grows_after_the_threshold_and_is_capped() {
        let w = |n| USER_POLICY.wait_ms(n);
        assert_eq!([w(1), w(4)], [0, 0]);
        assert_eq!([w(5), w(6), w(7), w(8)], [5_000, 10_000, 20_000, 40_000]);
        assert_eq!([w(9), w(100), w(u32::MAX)], [60_000, 60_000, 60_000]);
        assert_eq!(ADDRESS_POLICY.wait_ms(10), 30_000);
        assert_eq!(ADDRESS_POLICY.wait_ms(50), 900_000);
    }

    #[test]
    fn a_few_failures_are_free_then_the_user_is_refused_for_a_while() {
        let t = LoginThrottle::default();
        for i in 0..4 {
            fail(&t, T0 + i, "anna", "192.0.2.1");
            assert_eq!(t.check(T0 + i, "anna", ip("192.0.2.1")), Ok(()));
        }
        fail(&t, T0 + 4, "anna", "192.0.2.1");
        assert_eq!(t.check(T0 + 4, "anna", ip("192.0.2.1")), Err(5_000));
        assert_eq!(t.check(T0 + 3_004, "anna", ip("192.0.2.1")), Err(2_000));
        assert_eq!(t.check(T0 + 5_004, "anna", ip("192.0.2.1")), Ok(()));
    }

    #[test]
    fn the_user_limit_follows_the_name_across_addresses_and_ignores_case() {
        let t = LoginThrottle::default();
        for i in 0..5 {
            fail(&t, T0 + i, "Anna", &format!("192.0.2.{}", i + 1));
        }
        // A sixth address, never seen before, is still held back for this user name only.
        assert!(t.check(T0 + 10, " anna ", ip("198.51.100.9")).is_err());
        assert_eq!(t.check(T0 + 10, "bert", ip("198.51.100.9")), Ok(()));
    }

    #[test]
    fn the_address_limit_follows_the_address_across_user_names() {
        let t = LoginThrottle::default();
        for i in 0..10 {
            fail(&t, T0 + i, &format!("user{i}"), "192.0.2.1");
        }
        assert_eq!(
            t.check(T0 + 10, "somebody-else", ip("192.0.2.1")),
            Err(30_000 - 1)
        );
        assert_eq!(t.check(T0 + 10, "somebody-else", ip("192.0.2.2")), Ok(()));
    }

    #[test]
    fn an_attacker_cannot_keep_the_real_user_out_for_long() {
        let t = LoginThrottle::default();
        let mut now = T0;
        // The attacker hammers one account for an hour from changing addresses, as fast as allowed.
        for i in 0..3_600 {
            now += 1_000;
            let addr = format!("10.{}.{}.{}", i / 65_536, (i / 256) % 256, i % 256);
            if t.check(now, "admin", ip(&addr)).is_ok() {
                fail(&t, now, "admin", &addr);
            }
        }
        // The administrator is never refused for more than the cap of one minute.
        let wait = t.check(now, "admin", ip("203.0.113.5")).err().unwrap_or(0);
        assert!(wait <= 60_000, "{wait}");
        assert_eq!(t.check(now + 60_001, "admin", ip("203.0.113.5")), Ok(()));
    }

    #[test]
    fn failures_expire_with_the_window_and_success_clears_them() {
        let t = LoginThrottle::default();
        for i in 0..5 {
            fail(&t, T0 + i, "anna", "192.0.2.1");
        }
        assert!(t.check(T0 + 5, "anna", ip("192.0.2.1")).is_err());
        assert_eq!(
            t.check(T0 + WINDOW_MS + 10, "anna", ip("192.0.2.1")),
            Ok(())
        );

        for i in 0..5 {
            fail(&t, T0 + i, "bert", "192.0.2.2");
        }
        t.record_success("bert", ip("192.0.2.2"));
        assert_eq!(t.check(T0 + 5, "bert", ip("192.0.2.2")), Ok(()));
        // After the window a fresh failure starts counting from one again.
        fail(&t, T0 + 3 * WINDOW_MS, "anna", "192.0.2.1");
        assert_eq!(t.check(T0 + 3 * WINDOW_MS, "anna", ip("192.0.2.1")), Ok(()));
    }

    #[test]
    fn ipv6_is_grouped_by_prefix_and_unknown_addresses_share_a_bucket() {
        let t = LoginThrottle::default();
        for i in 0..10 {
            fail(
                &t,
                T0 + i,
                &format!("u{i}"),
                &format!("2001:db8:1:2::{i:x}"),
            );
        }
        assert!(t.check(T0 + 10, "x", ip("2001:db8:1:2:ffff::1")).is_err());
        assert_eq!(t.check(T0 + 10, "x", ip("2001:db8:1:3::1")), Ok(()));
        for i in 0..10 {
            t.record_failure(T0 + i, &format!("n{i}"), None);
        }
        assert!(t.check(T0 + 10, "x", None).is_err());
    }

    #[test]
    fn memory_stays_bounded_under_a_flood() {
        let t = LoginThrottle::default();
        for i in 0..(MAX_ENTRIES as i64 * 3) {
            t.record_failure(T0 + i, &format!("user-{i}"), None);
        }
        let state = t.state.lock().unwrap();
        assert!(state.users.len() <= MAX_ENTRIES, "{}", state.users.len());
    }

    #[test]
    fn long_user_names_are_cut_for_the_key() {
        assert_eq!(user_key(&"A".repeat(500)).len(), MAX_NAME_CHARS);
        assert_eq!(user_key("  Ärger "), "ärger");
    }

    #[test]
    fn the_client_address_comes_from_the_proxy_header_only_when_configured_and_valid() {
        let peer = ip("127.0.0.1");
        let mut h = HeaderMap::new();
        h.insert(
            "x-forwarded-for",
            HeaderValue::from_static("6.6.6.6, 203.0.113.7"),
        );
        h.insert("x-real-ip", HeaderValue::from_static("198.51.100.4"));
        assert_eq!(
            client_ip(peer, &h, None),
            peer,
            "no header configured: the peer"
        );
        assert_eq!(
            client_ip(peer, &h, Some(ClientIpHeader::XForwardedFor)),
            ip("203.0.113.7"),
            "the last entry is the proxy's"
        );
        assert_eq!(
            client_ip(peer, &h, Some(ClientIpHeader::XRealIp)),
            ip("198.51.100.4")
        );

        let mut bad = HeaderMap::new();
        bad.insert("x-forwarded-for", HeaderValue::from_static("not-an-ip"));
        assert_eq!(
            client_ip(peer, &bad, Some(ClientIpHeader::XForwardedFor)),
            peer
        );
        assert_eq!(
            client_ip(peer, &HeaderMap::new(), Some(ClientIpHeader::XRealIp)),
            peer
        );
        let mut v6 = HeaderMap::new();
        v6.insert("x-real-ip", HeaderValue::from_static("2001:db8::1"));
        assert_eq!(
            client_ip(peer, &v6, Some(ClientIpHeader::XRealIp)),
            ip("2001:db8::1")
        );
    }
}
