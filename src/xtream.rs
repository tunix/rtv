//! Real source ingestion: Xtream Codes via libsoup3 (GLib main loop,
//! mirroring `details.rs`), M3U over plain HTTP. Every request logs its
//! duration, byte size and status — the numbers we need to catch slowness
//! and spot oversized payloads while tuning the pipeline.
//!
//! Security rules (AGENTS.md): credentials ride inside request URLs only
//! in memory; they are NEVER logged, printed or persisted to disk.

use std::time::Instant;

use gtk::glib;
use serde_json::Value;
use soup::prelude::*;

/// The panel's origin filters User-Agents (AGENTS.md): CLI UAs get the
/// connection killed and Cloudflare reports HTTP 520. Every request MUST
/// present an app-like User-Agent.
pub const USER_AGENT: &str = "okhttp/4.9.3";

thread_local! {
    static SESSION: soup::Session = soup::Session::builder()
        .timeout(45)
        .user_agent(USER_AGENT)
        .build();
}

/// Instrumented GET: logs status, duration and payload size for every
/// request; `done` receives the body bytes or `None` on any transport /
/// HTTP failure. Callback runs on the GLib main loop, never blocking it.
fn get_bytes(url: String, tag: String, done: impl FnOnce(Option<Vec<u8>>) + 'static) {
    let started = Instant::now();
    let Ok(message) = soup::Message::new("GET", &url) else {
        tracing::error!(target: "rtv::fetch", tag, "invalid URL — request never issued");
        done(None);
        return;
    };
    if let Some(headers) = message.request_headers() {
        headers.append("User-Agent", USER_AGENT);
    }
    let status_message = message.clone();
    SESSION.with(|session| {
        session.send_and_read_async(
            &message,
            glib::Priority::DEFAULT,
            None::<&gio::Cancellable>,
            move |result| {
                let elapsed = started.elapsed();
                match result {
                    Ok(bytes) => {
                        let status = status_message.status_code();
                        let ms = elapsed.as_millis();
                        let kb = bytes.len() / 1024;
                        if (200..300).contains(&status) {
                            if ms > 3000 {
                                tracing::warn!(
                                    target: "rtv::fetch", tag, status, ms, kb,
                                    "slow response (>3s)"
                                );
                            } else {
                                tracing::info!(target: "rtv::fetch", tag, status, ms, kb, "ok");
                            }
                            done(Some(bytes.to_vec()));
                        } else {
                            tracing::error!(
                                target: "rtv::fetch", tag, status, ms,
                                "HTTP error — server or credentials rejected the request"
                            );
                            done(None);
                        }
                    }
                    Err(err) => {
                        tracing::error!(
                            target: "rtv::fetch", tag,
                            ms = elapsed.as_millis(),
                            error = %err,
                            "transport failure (timeout, DNS, refused…)"
                        );
                        done(None);
                    }
                }
            },
        );
    });
}

/// GET a JSON document with the same logging; `done` receives the parsed
/// value or `None`.
pub fn get_json(url: String, tag: String, done: impl FnOnce(Option<Value>) + 'static) {
    get_bytes(url, tag.clone(), move |bytes| {
        let value = bytes.and_then(|b| {
            serde_json::from_slice(&b)
                .map_err(|err| {
                    tracing::error!(
                        target: "rtv::fetch", tag,
                        error = %err,
                        "response was not valid JSON — wrong endpoint or panel?"
                    );
                })
                .ok()
        });
        done(value);
    });
}

/// GET a text document (M3U playlists).
pub fn get_text(url: String, tag: String, done: impl FnOnce(Option<String>) + 'static) {
    get_bytes(url, tag, move |bytes| {
        done(bytes.and_then(|b| String::from_utf8(b).ok()));
    });
}

/// Xtream account details for one source.
#[derive(Debug, Clone)]
pub struct XtreamCreds {
    /// Base like `http://host:port` (protocol + host + port only).
    pub base_url: String,
    pub username: String,
    pub password: String,
}

impl XtreamCreds {
    /// URL for one player_api.php action. NEVER log the returned string.
    pub fn api_url(&self, action: &str, extra: &str) -> String {
        format!(
            "{}/player_api.php?username={}&password={}&{}{}",
            self.base_url, self.username, self.password, action, extra
        )
    }

    /// URL of a playable stream. NEVER log the returned string.
    pub fn stream_url(&self, kind: crate::catalog::Kind, stream_id: i64, ext: &str) -> String {
        let section = match kind {
            crate::catalog::Kind::Live => "live",
            crate::catalog::Kind::Vod => "movie",
            crate::catalog::Kind::Series => "series",
        };
        format!(
            "{}/{}/{}/{}/{}.{}",
            self.base_url, self.username, self.password, section, stream_id, ext
        )
    }
}

/// `user_info` auth probe: `done(true)` means the account exists and the
/// credentials are accepted. Logs only statuses and timings.
pub fn auth_check(creds: &XtreamCreds, done: impl FnOnce(bool) + 'static) {
    let url = creds.api_url("", "");
    get_json(url, "user_info(auth)".to_string(), move |value| {
        let ok = value
            .as_ref()
            .and_then(|v| v.get("user_info"))
            .and_then(|u| u.get("auth"))
            .and_then(Value::as_u64)
            == Some(1);
        if ok {
            let exp_days = value
                .as_ref()
                .and_then(|v| v.get("user_info"))
                .and_then(|u| u.get("exp_date"))
                .and_then(Value::as_i64)
                .map(|exp| {
                    let secs = exp - (glib::DateTime::now_utc().unwrap().to_unix());
                    secs / 86_400
                })
                .unwrap_or(-1);
            tracing::info!(target: "rtv::auth", days_left = exp_days.max(0), "auth ok");
        } else {
            tracing::error!(target: "rtv::auth", "auth FAILED — check URL/username/password");
        }
        done(ok);
    });
}

/// M3U playlist ingestion: fetch + parse into (name, group) pairs. The
/// parser is deliberately tolerant: bad lines are skipped with a warning
/// count, parsing NEVER fails the whole ingest.
pub struct M3UEntry {
    pub name: String,
    pub group: String,
    /// The entry's playback URL (may embed credentials — never logged).
    /// Read by the ingest pipeline when building Items; the warning is
    /// silenced there since xtream.rs itself never reads it back.
    #[allow(dead_code)]
    pub url: String,
}

pub fn parse_m3u(text: &str) -> Vec<M3UEntry> {
    let mut out = Vec::new();
    let mut pending_name: Option<String> = None;
    let mut pending_group: Option<String> = None;
    let mut bad_attr_lines = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let rest = rest
                .split_once(',')
                .map(|(_, name)| name.trim())
                .unwrap_or("");
            pending_name = (!rest.is_empty()).then(|| rest.to_string());
            pending_group = None;
            // group-title="..." attribute scan.
            if let Some(idx) = line.find("group-title=\"") {
                let rest = &line[idx + "group-title=\"".len()..];
                if let Some(end) = rest.find('"') {
                    pending_group = Some(rest[..end].to_string());
                }
            }
            if pending_group.is_none() {
                pending_group = Some(String::new());
            }
        } else if line.starts_with('#') || line.is_empty() {
            continue;
        } else if let Some(name) = pending_name.take() {
            out.push(M3UEntry {
                name,
                group: pending_group.take().unwrap_or_default(),
                url: line.to_string(),
            });
        } else {
            bad_attr_lines += 1;
        }
    }
    if bad_attr_lines > 0 {
        tracing::warn!(target: "rtv::m3u", skipped = bad_attr_lines, "orphan URLs without EXTINF skipped");
    }
    tracing::info!(target: "rtv::m3u", count = out.len(), "playlist parsed");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_m3u_extracts_names_groups_and_urls() {
        let text = concat!(
            "#EXTM3U\n",
            "#EXTINF:-1 tvg-id=\"1\" group-title=\"Sports\",Match A\n",
            "http://s/1\n",
            "#EXTINF:-1 group-title=\"News\",Daily\n",
            "http://s/2\n"
        );
        let entries = parse_m3u(text);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "Match A");
        assert_eq!(entries[0].group, "Sports");
        assert_eq!(entries[0].url, "http://s/1");
        assert_eq!(entries[1].group, "News");
    }

    #[test]
    fn parse_m3u_skips_orphan_urls_without_extinf() {
        let text = "#EXTINF:-1 group-title=\"A\",X\nhttp://x\nhttp://orphan\n";
        let entries = parse_m3u(text);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "X");
    }

    #[test]
    fn parse_m3u_handles_empty_and_header_only() {
        assert!(parse_m3u("").is_empty());
        assert!(parse_m3u("#EXTM3U\n").is_empty());
    }

    #[test]
    fn parse_m3u_missing_group_becomes_empty_string() {
        let text = "#EXTINF:-1,No Group\nhttp://n\n";
        let entries = parse_m3u(text);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].group, "");
        // The ingest layer folds these into DEFAULT_GROUP; parse keeps raw.
    }
}
