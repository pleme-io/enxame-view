//! `enxame-view` — the shared view-model of the ENXAME BitTorrent suite.
//!
//! Every *face* — the web GUI (`enxame-web`, Leptos/Tela), the native
//! desktop GUI (`enxame-gui`, Dioxus), the terminal GUI (`enxame-tui`),
//! the CLI, and the daemon's RPC surface — renders the **same typed
//! snapshot**. The daemon (`enxamed`) is the single producer; the faces
//! are thin readers (`theory/ENXAME.md` L3 — "faces declare + observe
//! only"). This crate is the one place that snapshot is defined, so a
//! field added here is rendered everywhere by construction (the CSE
//! base + delta shape: one model, many faces).
//!
//! It is pure data — `serde` only, no I/O — so it compiles unchanged to
//! native *and* `wasm32` (the web GUI deserializes it in the browser).

#![forbid(unsafe_code)]
#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

/// A whole-daemon snapshot: every managed torrent plus aggregate rates.
/// This is the payload `enxamed`'s status RPC returns and the GUIs poll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// The torrents the daemon is managing, in display order.
    pub torrents: Vec<TorrentView>,
    /// Aggregate download rate across all torrents (bytes/sec).
    pub down_rate: u64,
    /// Aggregate upload rate across all torrents (bytes/sec).
    pub up_rate: u64,
}

impl Snapshot {
    /// An empty snapshot (the GUIs' initial / daemon-offline state).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            torrents: Vec::new(),
            down_rate: 0,
            up_rate: 0,
        }
    }

    /// How many of the managed torrents are fully downloaded.
    #[must_use]
    pub fn complete_count(&self) -> usize {
        self.torrents.iter().filter(|t| t.is_complete()).count()
    }
}

/// One torrent as the faces render it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TorrentView {
    /// Lower-case hex of the 20-byte v1 info-hash — the stable id.
    pub info_hash: String,
    /// Display name (from the metainfo, or the magnet's `dn`).
    pub name: String,
    /// Total content size in bytes.
    pub total_bytes: u64,
    /// Verified bytes on disk.
    pub have_bytes: u64,
    /// Lifecycle state.
    pub state: TorrentState,
    /// Current download rate (bytes/sec).
    pub down_rate: u64,
    /// Current upload rate (bytes/sec).
    pub up_rate: u64,
    /// Connected peers.
    pub peers: u32,
    /// Seeders known in the swarm (from the tracker / DHT).
    pub seeds: u32,
}

impl TorrentView {
    /// Completion as a `0.0..=1.0` fraction (saturating; `1.0` when the
    /// total is zero so an empty torrent reads as done, never NaN).
    #[must_use]
    pub fn fraction(&self) -> f64 {
        if self.total_bytes == 0 {
            return 1.0;
        }
        // Saturate: a daemon over-reporting `have` never yields > 1.0.
        let have = self.have_bytes.min(self.total_bytes);
        have as f64 / self.total_bytes as f64
    }

    /// Whole-number completion percent (`0..=100`).
    #[must_use]
    pub fn percent(&self) -> u8 {
        // `fraction()` is in `0.0..=1.0`, so this is always in range.
        (self.fraction() * 100.0) as u8
    }

    /// Is every piece present?
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.have_bytes >= self.total_bytes && self.total_bytes > 0
            || self.state == TorrentState::Seeding
    }
}

/// The lifecycle state of a torrent — a closed sum (every face matches it
/// exhaustively, so a new state can never be silently un-rendered).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TorrentState {
    /// Resolving metadata from peers (magnet, pre-metainfo).
    Metadata,
    /// Checking existing files against the piece hashes.
    Checking,
    /// Actively downloading.
    Downloading,
    /// Complete; uploading to peers.
    Seeding,
    /// Paused by the operator.
    Paused,
    /// Stopped on an error (the message travels in [`TorrentView`] logs
    /// elsewhere; this is the rendered badge).
    Errored,
}

impl TorrentState {
    /// A short human label for the state badge.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::Checking => "checking",
            Self::Downloading => "downloading",
            Self::Seeding => "seeding",
            Self::Paused => "paused",
            Self::Errored => "error",
        }
    }
}

/// Render `bytes` as a compact human string (`"1.5 GiB"`), allocation-
/// light and identical across every face so sizes never disagree.
#[must_use]
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        let mut s = String::new();
        push_u64(&mut s, bytes);
        s.push(' ');
        s.push_str(UNITS[0]);
        return s;
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    // One decimal place, rounded — typed assembly, never `format!`.
    let tenths = (value * 10.0 + 0.5) as u64;
    let mut s = String::new();
    push_u64(&mut s, tenths / 10);
    s.push('.');
    push_u64(&mut s, tenths % 10);
    s.push(' ');
    s.push_str(UNITS[unit]);
    s
}

/// Render `bytes`/sec as a rate string (`"1.5 GiB/s"`).
#[must_use]
pub fn human_rate(bytes_per_sec: u64) -> String {
    let mut s = human_bytes(bytes_per_sec);
    s.push_str("/s");
    s
}

/// Push a `u64`'s decimal digits onto `out` without `format!`.
fn push_u64(out: &mut String, mut n: u64) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    // Safe: only ASCII digits were written.
    out.push_str(core::str::from_utf8(&buf[i..]).unwrap_or("0"));
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    fn view(have: u64, total: u64, state: TorrentState) -> TorrentView {
        TorrentView {
            info_hash: "0".repeat(40),
            name: "ubuntu.iso".to_string(),
            total_bytes: total,
            have_bytes: have,
            state,
            down_rate: 0,
            up_rate: 0,
            peers: 0,
            seeds: 0,
        }
    }

    #[test]
    fn fraction_and_percent() {
        let v = view(512, 1024, TorrentState::Downloading);
        assert!((v.fraction() - 0.5).abs() < 1e-9);
        assert_eq!(v.percent(), 50);
    }

    #[test]
    fn fraction_saturates_and_never_nans() {
        assert_eq!(view(0, 0, TorrentState::Seeding).fraction(), 1.0); // empty = done
        assert_eq!(view(9999, 1024, TorrentState::Seeding).fraction(), 1.0); // over-report clamps
    }

    #[test]
    fn complete_count_uses_state_and_bytes() {
        let snap = Snapshot {
            torrents: vec![
                view(1024, 1024, TorrentState::Seeding),
                view(10, 1024, TorrentState::Downloading),
            ],
            down_rate: 100,
            up_rate: 50,
        };
        assert_eq!(snap.complete_count(), 1);
    }

    #[test]
    fn human_bytes_is_stable() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(1024 * 1024), "1.0 MiB");
        assert_eq!(human_rate(1024), "1.0 KiB/s");
    }

    #[test]
    fn snapshot_round_trips_json() {
        let snap = Snapshot {
            torrents: vec![view(512, 1024, TorrentState::Downloading)],
            down_rate: 200,
            up_rate: 75,
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snap, back);
        // State serializes as snake_case for a stable wire contract.
        assert!(json.contains("\"downloading\""));
    }

    #[test]
    fn state_labels_are_exhaustive() {
        for s in [
            TorrentState::Metadata,
            TorrentState::Checking,
            TorrentState::Downloading,
            TorrentState::Seeding,
            TorrentState::Paused,
            TorrentState::Errored,
        ] {
            assert!(!s.label().is_empty());
        }
    }
}
