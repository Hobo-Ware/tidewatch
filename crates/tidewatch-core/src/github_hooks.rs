//! The repos whose GitHub webhook deliveries the listener takes: a path token and a secret per
//! repo, the signature check, and the message each good delivery becomes.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, PoisonError};

use serde_json::Value;

use crate::events::{RepoEvent, RepoMessage};
use crate::github_hook::{hit, ping_hook_id, verify};

pub const PATH_PREFIX: &str = "/github/";

/// Where a repo's deliveries go and the secret they are signed with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoHook {
    pub url: String,
    pub secret: String,
}

/// How a delivery was answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Taken,
    BadSignature,
    BadRequest(&'static str),
}

struct Entry {
    repo: String,
    secret: String,
}

pub(crate) struct GithubHooks {
    by_token: Mutex<HashMap<String, Entry>>,
    events: Sender<RepoMessage>,
}

impl GithubHooks {
    pub(crate) fn new() -> (Self, Receiver<RepoMessage>) {
        let (events, receiver) = channel();
        (Self { by_token: Mutex::new(HashMap::new()), events }, receiver)
    }

    pub(crate) fn register(&self, repo: &str, token: String, secret: String) {
        let mut tokens = self.by_token.lock().unwrap_or_else(PoisonError::into_inner);
        tokens.retain(|_, entry| !entry.repo.eq_ignore_ascii_case(repo));
        tokens.insert(token, Entry { repo: repo.to_owned(), secret });
    }

    pub(crate) fn unregister(&self, repo: &str) {
        let mut tokens = self.by_token.lock().unwrap_or_else(PoisonError::into_inner);
        tokens.retain(|_, entry| !entry.repo.eq_ignore_ascii_case(repo));
    }

    pub(crate) fn known(&self, token: &str) -> bool {
        self.by_token.lock().unwrap_or_else(PoisonError::into_inner).contains_key(token)
    }

    pub(crate) fn deliver(
        &self,
        token: &str,
        event: Option<&str>,
        signature: Option<&str>,
        body: &[u8],
    ) -> Answer {
        let found = {
            let tokens = self.by_token.lock().unwrap_or_else(PoisonError::into_inner);
            tokens.get(token).map(|entry| (entry.repo.clone(), entry.secret.clone()))
        };
        let Some((repo, secret)) = found else { return Answer::BadSignature };
        if !signature.is_some_and(|header| verify(&secret, body, header)) {
            return Answer::BadSignature;
        }
        let Some(event) = event else { return Answer::BadRequest("no X-GitHub-Event") };
        let Ok(payload) = serde_json::from_slice::<Value>(body) else {
            return Answer::BadRequest("the body is not JSON");
        };
        let news = if event == "ping" {
            ping_hook_id(&payload).map(|hook_id| RepoEvent::Ping { hook_id })
        } else {
            hit(event, &payload).map(RepoEvent::News)
        };
        if let Some(event) = news {
            let _ = self.events.send(RepoMessage { repo, event });
        }
        Answer::Taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github_hook::signature_header;
    use serde_json::json;

    fn hooks() -> (GithubHooks, Receiver<RepoMessage>) {
        let (hooks, receiver) = GithubHooks::new();
        hooks.register("Acme/web", "tok".into(), "s3cret".into());
        (hooks, receiver)
    }

    fn signed(body: &Value) -> (Vec<u8>, String) {
        let bytes = body.to_string().into_bytes();
        let header = signature_header("s3cret", &bytes);
        (bytes, header)
    }

    #[test]
    fn a_signed_ping_and_a_signed_pull_request_event_become_messages_for_the_repo() {
        let (hooks, receiver) = hooks();
        let (ping, sig) = signed(&json!({"zen": "x", "hook_id": 630}));
        assert_eq!(hooks.deliver("tok", Some("ping"), Some(&sig), &ping), Answer::Taken);
        let (pull, sig) = signed(&json!({"pull_request": {"number": 4, "head": {"ref": "b"}}}));
        assert_eq!(hooks.deliver("tok", Some("pull_request"), Some(&sig), &pull), Answer::Taken);
        let got: Vec<_> = receiver.try_iter().collect();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].repo, "Acme/web");
        assert_eq!(got[0].event, RepoEvent::Ping { hook_id: 630 });
        assert!(matches!(&got[1].event, RepoEvent::News(hit) if hit.numbers == [4]));
    }

    #[test]
    fn a_bad_unknown_or_missing_signature_is_refused_and_sends_nothing() {
        let (hooks, receiver) = hooks();
        let (body, sig) = signed(&json!({"zen": "x", "hook_id": 1}));
        let wrong = signature_header("other", &body);
        let cases = [
            ("tok", Some(wrong.as_str())),
            ("tok", None),
            ("tok", Some("sha256=00")),
            ("nope", Some(sig.as_str())),
        ];
        for (token, signature) in cases {
            assert_eq!(
                hooks.deliver(token, Some("ping"), signature, &body),
                Answer::BadSignature,
                "{token} {signature:?}"
            );
        }
        assert!(receiver.try_iter().next().is_none());
    }

    #[test]
    fn a_signed_delivery_without_an_event_or_json_is_a_bad_request_and_an_ignored_event_is_taken() {
        let (hooks, receiver) = hooks();
        let (body, sig) = signed(&json!({"a": 1}));
        assert!(matches!(hooks.deliver("tok", None, Some(&sig), &body), Answer::BadRequest(_)));
        let raw = b"not json";
        let sig = signature_header("s3cret", raw);
        assert!(matches!(
            hooks.deliver("tok", Some("push"), Some(&sig), raw),
            Answer::BadRequest(_)
        ));
        let (star, sig) = signed(&json!({"action": "created"}));
        assert_eq!(hooks.deliver("tok", Some("star"), Some(&sig), &star), Answer::Taken);
        assert!(receiver.try_iter().next().is_none());
    }

    #[test]
    fn registering_a_repo_again_revokes_the_old_token_and_unregistering_removes_it() {
        let (hooks, _receiver) = hooks();
        hooks.register("acme/WEB", "tok2".into(), "x".into());
        assert!(!hooks.known("tok") && hooks.known("tok2"));
        hooks.unregister("ACME/web");
        assert!(!hooks.known("tok2"));
    }
}
