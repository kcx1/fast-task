//! Request and response bodies of the share's `/api/v1`, shared by the server
//! and the browser client so the two can't drift apart.

use serde::{Deserialize, Serialize};

use crate::database::models::{Priority, TaskStatus};

/// Header carrying the share token. Reads also accept `?t=`, since a browser
/// `EventSource` can't set headers; writes need the header.
pub const TOKEN_HEADER: &str = "x-share-token";

/// Header naming who makes a write, for the activity log. Percent-encoded
/// (header values must be ASCII); see [`encode_name`] / [`decode_name`].
pub const NAME_HEADER: &str = "x-share-name";

/// Longest name kept, in characters.
pub const NAME_MAX: usize = 40;

/// Percent-encode a name for [`NAME_HEADER`].
pub fn encode_name(name: &str) -> String {
    name.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~ ".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Decode [`NAME_HEADER`]; trims and caps it. `None` if empty.
pub fn decode_name(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let name: String = String::from_utf8_lossy(&out)
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(NAME_MAX)
        .collect();
    (!name.is_empty()).then_some(name)
}

/// `GET /share`: what this share lets browsers do.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ShareInfo {
    pub can_edit: bool,
    /// How the desktop's own edits are labelled in the activity log.
    pub host_name: String,
}

/// Body of `POST /task`. The server fills in everything else.
#[derive(Serialize, Deserialize)]
pub struct NewTask {
    pub title: String,
}

/// Body of `PATCH /task/{id}`: only the fields present change.
#[derive(Serialize, Deserialize, Default)]
pub struct TaskPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
}

/// Body of `POST /task/{id}/annotations`.
#[derive(Serialize, Deserialize)]
pub struct NewNote {
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_through_the_header() {
        for name in ["Sam", "José Ñúñez", "李雷", "a%b", "x y"] {
            let encoded = encode_name(name);
            assert!(encoded.is_ascii(), "{encoded}");
            assert_eq!(decode_name(&encoded).as_deref(), Some(name));
        }
        assert_eq!(decode_name("   "), None);
        assert_eq!(decode_name("%zz").as_deref(), Some("%zz"));
        assert_eq!(decode_name(&"a".repeat(100)).unwrap().len(), NAME_MAX);
    }
}
