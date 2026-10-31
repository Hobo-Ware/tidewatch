//! A GitHub webhook delivery, pure: the HMAC-SHA256 signature check (the `hmac` crate) and who an event is about.

use std::fmt::Write as _;

use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

/// The event types the forwarder asks GitHub for, comma separated for `--events`.
pub const EVENTS: &str = "pull_request,pull_request_review,pull_request_review_comment,\
issue_comment,issues,check_suite,check_run,workflow_run,push";

/// What an event names: the pull requests and branches whose tabs should look at GitHub now, and
/// the plain issues whose links should.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hit {
    pub event: String,
    pub numbers: Vec<u64>,
    pub branches: Vec<String>,
    pub issues: Vec<u64>,
}

#[must_use]
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut mac = mac_for(key);
    mac.update(message);
    mac.finalize().into_bytes().into()
}

fn mac_for(key: &[u8]) -> Hmac<Sha256> {
    Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts a key of any length")
}

/// The `X-Hub-Signature-256` value GitHub sends for `body`.
#[must_use]
pub fn signature_header(secret: &str, body: &[u8]) -> String {
    format!("sha256={}", hex(&hmac_sha256(secret.as_bytes(), body)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len()).step_by(2).map(|at| u8::from_str_radix(&text[at..at + 2], 16).ok()).collect()
}

/// Whether `header` is the signature of `body` under `secret`, compared in constant time.
#[must_use]
pub fn verify(secret: &str, body: &[u8], header: &str) -> bool {
    let Some(given) = header.trim().strip_prefix("sha256=").and_then(unhex) else {
        return false;
    };
    let mut mac = mac_for(secret.as_bytes());
    mac.update(body);
    mac.verify_slice(&given).is_ok()
}

fn number(value: &Value) -> Option<u64> {
    value.as_u64().filter(|n| *n > 0)
}

fn numbers_of(list: &Value) -> Vec<u64> {
    list.as_array().into_iter().flatten().filter_map(|pr| number(&pr["number"])).collect()
}

fn branch(value: &Value) -> Option<String> {
    value.as_str().filter(|name| !name.is_empty()).map(str::to_owned)
}

/// Who a delivery of `event` is about, or `None` for an event Tidewatch does not react to.
#[must_use]
pub fn hit(event: &str, payload: &Value) -> Option<Hit> {
    let mut issues = Vec::new();
    let (numbers, branches) = match event {
        "pull_request" | "pull_request_review" | "pull_request_review_comment" => {
            let pull = &payload["pull_request"];
            (
                number(&pull["number"]).into_iter().collect(),
                branch(&pull["head"]["ref"]).into_iter().collect(),
            )
        }
        "issue_comment" => {
            let issue = &payload["issue"];
            if issue["pull_request"].is_null() {
                issues = number(&issue["number"]).into_iter().collect();
                (Vec::new(), Vec::new())
            } else {
                (number(&issue["number"]).into_iter().collect(), Vec::new())
            }
        }
        "issues" => {
            issues = number(&payload["issue"]["number"]).into_iter().collect();
            (Vec::new(), Vec::new())
        }
        "check_suite" => {
            let suite = &payload["check_suite"];
            (
                numbers_of(&suite["pull_requests"]),
                branch(&suite["head_branch"]).into_iter().collect(),
            )
        }
        "check_run" => {
            let run = &payload["check_run"];
            (
                numbers_of(&run["pull_requests"]),
                branch(&run["check_suite"]["head_branch"]).into_iter().collect(),
            )
        }
        "workflow_run" => {
            let run = &payload["workflow_run"];
            (numbers_of(&run["pull_requests"]), branch(&run["head_branch"]).into_iter().collect())
        }
        "push" => {
            let name = payload["ref"].as_str()?.strip_prefix("refs/heads/")?;
            (Vec::new(), vec![name.to_owned()])
        }
        _ => return None,
    };
    Some(Hit { event: event.to_owned(), numbers, branches, issues })
}

/// The id of the webhook a `ping` delivery came through.
#[must_use]
pub fn ping_hook_id(payload: &Value) -> Option<u64> {
    number(&payload["hook_id"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hmac_matches_the_rfc_4231_vectors() {
        let cases: [(Vec<u8>, &[u8], &str); 4] = [
            (
                vec![0x0b; 20],
                b"Hi There",
                "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
            ),
            (
                b"Jefe".to_vec(),
                b"what do ya want for nothing?",
                "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
            ),
            (
                vec![0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First",
                "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
            ),
            (
                vec![0xaa; 20],
                &[0xdd; 50],
                "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
            ),
        ];
        for (key, message, want) in cases {
            assert_eq!(hex(&hmac_sha256(&key, message)), want, "key of {} bytes", key.len());
        }
    }

    #[test]
    fn the_example_signature_from_githubs_documentation_verifies() {
        let header = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
        assert_eq!(signature_header("It's a Secret to Everybody", b"Hello, World!"), header);
        assert!(verify("It's a Secret to Everybody", b"Hello, World!", header));
        assert!(!verify("It's a Secret to Everybody", b"Hello, World?", header));
    }

    #[test]
    fn a_signature_verifies_only_for_the_same_secret_and_body() {
        let header = signature_header("s3cret", b"{\"a\":1}");
        assert!(header.starts_with("sha256="));
        assert!(verify("s3cret", b"{\"a\":1}", &header));
        assert!(!verify("other", b"{\"a\":1}", &header));
        assert!(!verify("s3cret", b"{\"a\":2}", &header));
        assert!(!verify("s3cret", b"{\"a\":1}", ""));
        assert!(!verify("s3cret", b"{\"a\":1}", "sha256="));
        assert!(!verify("s3cret", b"{\"a\":1}", &header.replace("sha256=", "sha1=")));
        assert!(!verify("s3cret", b"{\"a\":1}", &header[..header.len() - 1]));
    }

    #[test]
    fn each_event_names_its_pull_requests_and_branches() {
        let cases = [
            (
                "pull_request",
                json!({"pull_request": {"number": 7, "head": {"ref": "agent/calm"}}}),
                Some((vec![7], vec!["agent/calm"])),
            ),
            (
                "pull_request_review",
                json!({"pull_request": {"number": 7, "head": {"ref": "agent/calm"}}}),
                Some((vec![7], vec!["agent/calm"])),
            ),
            (
                "pull_request_review_comment",
                json!({"pull_request": {"number": 8, "head": {"ref": "x"}}}),
                Some((vec![8], vec!["x"])),
            ),
            (
                "issue_comment",
                json!({"issue": {"number": 9, "pull_request": {"url": "u"}}}),
                Some((vec![9], vec![])),
            ),
            ("issue_comment", json!({"issue": {"number": 9}}), Some((vec![], vec![]))),
            (
                "check_suite",
                json!({"check_suite": {"head_branch": "b", "pull_requests": [{"number": 3}, {"number": 4}]}}),
                Some((vec![3, 4], vec!["b"])),
            ),
            (
                "check_run",
                json!({"check_run": {"check_suite": {"head_branch": "b"}, "pull_requests": []}}),
                Some((vec![], vec!["b"])),
            ),
            (
                "workflow_run",
                json!({"workflow_run": {"head_branch": "b", "pull_requests": [{"number": 5}]}}),
                Some((vec![5], vec!["b"])),
            ),
            ("push", json!({"ref": "refs/heads/feat/x"}), Some((vec![], vec!["feat/x"]))),
            ("push", json!({"ref": "refs/tags/v1"}), None),
            ("issues", json!({"issue": {"number": 1}}), Some((vec![], vec![]))),
            ("star", json!({}), None),
        ];
        for (event, payload, want) in cases {
            let got = hit(event, &payload).map(|h| (h.numbers, h.branches));
            let want = want.map(|(n, b)| (n, b.into_iter().map(str::to_owned).collect::<Vec<_>>()));
            assert_eq!(got, want, "{event} {payload}");
        }
    }

    #[test]
    fn issue_events_and_comments_on_a_plain_issue_name_the_issue_and_no_pull_request() {
        let cases = [
            ("issues", json!({"action": "closed", "issue": {"number": 623}}), vec![623]),
            ("issues", json!({"action": "reopened", "issue": {"number": 4}}), vec![4]),
            ("issue_comment", json!({"issue": {"number": 9}}), vec![9]),
            (
                "issue_comment",
                json!({"issue": {"number": 9, "pull_request": {"url": "u"}}}),
                vec![],
            ),
            ("issues", json!({"issue": {}}), vec![]),
        ];
        for (event, payload, want) in cases {
            let got = hit(event, &payload).unwrap();
            assert_eq!(got.issues, want, "{event} {payload}");
            assert_eq!(got.branches, Vec::<String>::new(), "{event} {payload}");
        }
        let pull = hit("issue_comment", &json!({"issue": {"number": 9, "pull_request": {}}}));
        assert_eq!(
            pull.unwrap().numbers,
            [9],
            "a comment on a pull request names the pull request"
        );
    }

    #[test]
    fn the_forwarder_asks_github_for_issue_events() {
        assert!(EVENTS.split(',').any(|event| event == "issues"));
    }

    #[test]
    fn a_ping_carries_the_hook_id() {
        assert_eq!(ping_hook_id(&json!({"zen": "x", "hook_id": 630})), Some(630));
        assert_eq!(ping_hook_id(&json!({"zen": "x"})), None);
        assert_eq!(ping_hook_id(&json!({"hook_id": 0})), None);
    }

    #[test]
    fn every_forwarded_event_is_one_the_parser_knows() {
        for event in EVENTS.split(',') {
            assert!(
                hit(
                    event,
                    &json!({"ref": "refs/heads/a", "issue": {"number": 1, "pull_request": {}}})
                )
                .is_some(),
                "{event}"
            );
        }
    }
}
