// Copyright 2026 Salesforce, Inc. All rights reserved.
//! Pure delegation-chain logic (no PDK imports) — fully unit-testable.

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Delegation context carried on the request header (JSON).
/// `agent`/`parent` are part of the wire schema; `parent` is retained for
/// provenance/telemetry even though enforcement keys on root+agent+depth.
#[derive(Deserialize, Debug, Default, Clone)]
#[allow(dead_code)]
pub struct Delegation {
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub depth: i64,
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub parent: String,
}

/// Parse the delegation header value. An absent/blank header ⇒ a root call
/// (depth 0). Malformed JSON ⇒ None (caller decides posture).
pub fn parse(header: Option<&str>) -> Option<Delegation> {
    match header {
        None => Some(Delegation::default()), // root call
        Some(h) if h.trim().is_empty() => Some(Delegation::default()),
        Some(h) => serde_json::from_str::<Delegation>(h).ok(),
    }
}

/// True when `depth` breaches the ceiling.
pub fn depth_exceeded(depth: i64, max_depth: i64) -> bool {
    depth > max_depth
}

pub fn hash_id(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Distinct sub-agent ids seen per root within a window.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Seen {
    pub id: u64,
    pub ts: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct FanoutState {
    pub seen: Vec<Seen>,
}

/// Prune entries outside the window, record `agent_id` (dedup, refresh ts), cap
/// the buffer, and return the updated state and the distinct count in-window.
pub fn record_distinct(
    mut st: FanoutState,
    agent_id: u64,
    now: u64,
    window_ms: u64,
    cap: usize,
) -> (FanoutState, usize) {
    st.seen.retain(|e| now.saturating_sub(e.ts) <= window_ms);
    if let Some(e) = st.seen.iter_mut().find(|e| e.id == agent_id) {
        e.ts = now;
    } else {
        st.seen.push(Seen { id: agent_id, ts: now });
    }
    if st.seen.len() > cap {
        let excess = st.seen.len() - cap;
        st.seen.drain(0..excess);
    }
    let distinct = st.seen.len();
    (st, distinct)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_absent_is_root() {
        let d = parse(None).unwrap();
        assert_eq!(d.depth, 0);
        assert_eq!(d.root, "");
    }

    #[test]
    fn parse_json_header() {
        let d = parse(Some(r#"{"root":"r1","depth":3,"agent":"a3","parent":"a2"}"#)).unwrap();
        assert_eq!(d.depth, 3);
        assert_eq!(d.root, "r1");
        assert_eq!(d.agent, "a3");
        assert_eq!(d.parent, "a2");
    }

    #[test]
    fn parse_malformed_is_none() {
        assert!(parse(Some("not-json")).is_none());
    }

    #[test]
    fn depth_ceiling() {
        assert!(!depth_exceeded(5, 5));
        assert!(depth_exceeded(6, 5));
        assert!(depth_exceeded(1, 0)); // maxDepth 0 = only root
        assert!(!depth_exceeded(0, 0));
    }

    #[test]
    fn fanout_counts_distinct_agents() {
        let mut st = FanoutState::default();
        let mut last = 0;
        for a in ["a1", "a2", "a3", "a2", "a1"] {
            let (ns, c) = record_distinct(st, hash_id(a), 1000, 60_000, 256);
            st = ns;
            last = c;
        }
        assert_eq!(last, 3); // a1,a2,a3 distinct
    }

    #[test]
    fn fanout_prunes_window() {
        let (st, c1) = record_distinct(FanoutState::default(), hash_id("a1"), 0, 10_000, 256);
        assert_eq!(c1, 1);
        let (_st, c2) = record_distinct(st, hash_id("a2"), 20_000, 10_000, 256);
        assert_eq!(c2, 1); // a1 aged out of the 10s window
    }
}
