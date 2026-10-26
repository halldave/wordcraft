//! The host bridge's decisions, kept free of `web_sys` so they are tested natively: which parent
//! origin the bridge may talk to, and which saves the parent has confirmed.

use std::collections::BTreeMap;

/// How long the parent has to answer `wordcraft:save` with `wordcraft:saved` (milliseconds).
pub const SAVE_ACK_TIMEOUT_MS: f64 = 30_000.0;

/// The origin the bridge posts to and hears from, or `None` for no bridge (saves download).
///
/// - `own`: this page's origin.
/// - `parent`: the parent window's origin, `None` when reading it threw (a cross-origin parent).
/// - `allowed`: the `?host_origin=` query value, an origin the embedding page vouches for.
///
/// A same-origin parent is accepted. A cross-origin parent is accepted only when `allowed` is a
/// valid URL origin (`scheme://host[:port]`, nothing else); messages then go to that exact origin,
/// so the browser drops them if the parent is anything else. Opaque origins (`"null"`) never match.
pub fn target_origin(own: &str, parent: Option<&str>, allowed: Option<&str>) -> Option<String> {
    if let Some(parent) = parent
        && parent == own
        && is_origin(own)
    {
        return Some(own.to_string());
    }
    let allowed = allowed?.trim();
    is_origin(allowed).then(|| allowed.to_string())
}

/// True for a serialised URL origin: `http(s)://host[:port]` with no path, query, fragment or
/// credentials.
pub fn is_origin(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once("://") else { return false };
    if !matches!(scheme, "http" | "https") || rest.is_empty() {
        return false;
    }
    // IPv6 literals aren't accepted; name the host instead.
    if rest.contains(['/', '?', '#', '@', ' ', '\\', '[', ']']) {
        return false;
    }
    let (host, port) = match rest.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (rest, None),
    };
    if host.is_empty() {
        return false;
    }
    match port {
        Some(p) => p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u16>().is_ok(),
        None => true,
    }
}

/// Saves posted to the parent and not yet confirmed, keyed by the app's save ticket.
#[derive(Debug, Default)]
pub struct SaveAcks {
    /// Request id → (file name, deadline in ms).
    pending: BTreeMap<u64, (String, f64)>,
}

impl SaveAcks {
    /// Record save `id` posted at `now`.
    pub fn posted_as(&mut self, id: u64, name: &str, now: f64) {
        self.pending.insert(id, (name.to_string(), now + SAVE_ACK_TIMEOUT_MS));
    }

    /// The parent confirmed `id`. False when no such save is waiting (an unknown id, or one that
    /// already timed out and was reported failed).
    pub fn saved(&mut self, id: u64) -> bool {
        self.pending.remove(&id).is_some()
    }

    /// Saves past their deadline at `now`, removed and returned as `(id, name)`.
    pub fn expire(&mut self, now: f64) -> Vec<(u64, String)> {
        let late: Vec<u64> = self.pending.iter().filter(|(_, (_, deadline))| now >= *deadline).map(|(id, _)| *id).collect();
        late.into_iter().filter_map(|id| self.pending.remove(&id).map(|(name, _)| (id, name))).collect()
    }

    /// Whether any save is still waiting for the parent (the document is reported unsaved).
    pub fn waiting(&self) -> bool {
        !self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: &str = "https://app.example";

    #[test]
    fn same_origin_parent_enables_the_bridge() {
        assert_eq!(target_origin(OWN, Some(OWN), None).as_deref(), Some(OWN));
    }

    #[test]
    fn cross_origin_parent_disables_the_bridge() {
        // Reading a cross-origin parent's origin throws (None) or names another origin.
        assert_eq!(target_origin(OWN, None, None), None);
        assert_eq!(target_origin(OWN, Some("https://evil.example"), None), None);
        assert_eq!(target_origin(OWN, Some("null"), None), None);
        assert_eq!(target_origin("null", Some("null"), None), None);
    }

    #[test]
    fn host_origin_parameter_must_be_an_exact_origin() {
        let files = "https://files.example:8443";
        assert_eq!(target_origin(OWN, None, Some(files)).as_deref(), Some(files));
        for bad in
            ["*", "null", "files.example", "https://files.example/", "https://a@files.example", "javascript://x", "https://", "https://x:99999"]
        {
            assert_eq!(target_origin(OWN, None, Some(bad)), None, "{bad}");
        }
    }

    #[test]
    fn a_save_stays_unsaved_until_its_own_confirmation() {
        let mut acks = SaveAcks::default();
        acks.posted_as(7, "Document.docx", 0.0);
        assert!(acks.waiting(), "the document is reported unsaved until the parent answers");
        assert!(!acks.saved(6), "an unknown id confirms nothing");
        assert!(acks.waiting());
        assert!(acks.saved(7));
        assert!(!acks.waiting());
        assert!(!acks.saved(7), "a confirmation counts once");
    }

    #[test]
    fn an_unanswered_save_times_out() {
        let mut acks = SaveAcks::default();
        acks.posted_as(3, "Document.docx", 1_000.0);
        assert!(acks.expire(1_000.0 + SAVE_ACK_TIMEOUT_MS - 1.0).is_empty());
        assert_eq!(acks.expire(1_000.0 + SAVE_ACK_TIMEOUT_MS), vec![(3, "Document.docx".to_string())]);
        assert!(!acks.saved(3), "a late confirmation is ignored: the save was reported failed");
        assert!(!acks.waiting());
    }
}
