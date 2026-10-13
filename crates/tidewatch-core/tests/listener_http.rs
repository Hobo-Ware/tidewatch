//! The listener on a real random port, driven by a tiny raw HTTP client: auth, hooks, MCP over HTTP,
//! keep-alive, chunked bodies, `Expect: 100-continue`, size caps and concurrent tabs.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use serde_json::{Value, json};
use tidewatch_core::events::{TabEvent, TabMessage};
use tidewatch_core::launch::HookListener;
use tidewatch_core::listener::{Listener, TabRegistration};
use tidewatch_core::paths::TidewatchPaths;
use tidewatch_core::status::HookKind;

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("body {:?}: {e}", self.body))
    }
}

struct Client {
    stream: TcpStream,
    carry: Vec<u8>,
}

impl Client {
    fn connect(addr: SocketAddr) -> Self {
        let stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        Self { stream, carry: Vec::new() }
    }

    fn send_raw(&mut self, bytes: &[u8]) {
        self.stream.write_all(bytes).unwrap();
    }

    fn read_reply(&mut self) -> Option<Reply> {
        loop {
            if let Some(end) = find(&self.carry, b"\r\n\r\n") {
                let head = String::from_utf8(self.carry[..end].to_vec()).unwrap();
                let mut lines = head.lines();
                let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
                let headers: Vec<(String, String)> = lines
                    .map(|l| {
                        let (k, v) = l.split_once(':').unwrap();
                        (k.trim().to_owned(), v.trim().to_owned())
                    })
                    .collect();
                let length: usize = headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                    .map_or(0, |(_, v)| v.parse().unwrap());
                while self.carry.len() < end + 4 + length {
                    if !self.fill() {
                        return None;
                    }
                }
                let body =
                    String::from_utf8(self.carry[end + 4..end + 4 + length].to_vec()).unwrap();
                self.carry.drain(..end + 4 + length);
                return Some(Reply { status, headers, body });
            }
            if !self.fill() {
                return None;
            }
        }
    }

    fn fill(&mut self) -> bool {
        let mut chunk = [0u8; 4096];
        match self.stream.read(&mut chunk) {
            Ok(0) | Err(_) => false,
            Ok(n) => {
                self.carry.extend_from_slice(&chunk[..n]);
                true
            }
        }
    }

    fn request(&mut self, method: &str, path: &str, token: Option<&str>, body: &str) -> Reply {
        let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
        let text = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        self.send_raw(text.as_bytes());
        self.read_reply().expect("the server answered")
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

struct Harness {
    listener: Listener,
    events: Receiver<TabMessage>,
    repaints: Arc<AtomicUsize>,
    paths: TidewatchPaths,
    _home: tempfile::TempDir,
}

const REPO: &str = "/work/web";

impl Harness {
    fn start() -> Self {
        let home = tempfile::tempdir().unwrap();
        let paths = TidewatchPaths::new(home.path());
        let repaints = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&repaints);
        let repaint = Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let (listener, events) = Listener::start(paths.clone(), repaint).unwrap();
        Self { listener, events, repaints, paths, _home: home }
    }

    fn addr(&self) -> SocketAddr {
        self.listener.base_url().strip_prefix("http://").unwrap().parse().unwrap()
    }

    fn client(&self) -> Client {
        Client::connect(self.addr())
    }

    fn register(&self, tab_id: u64, tab: &str) -> HookListener {
        self.register_generation(tab_id, tab, 0)
    }

    fn register_generation(&self, tab_id: u64, tab: &str, generation: u64) -> HookListener {
        self.listener.register_tab(TabRegistration {
            tab_id,
            agent: "claude".into(),
            repo: PathBuf::from(REPO),
            tab: tab.into(),
            generation,
        })
    }

    fn next_event(&self) -> TabMessage {
        self.events.recv_timeout(Duration::from_secs(5)).expect("an event arrived")
    }

    fn no_more_events(&self) {
        assert_eq!(self.events.try_iter().collect::<Vec<_>>(), Vec::new());
    }

    fn notes_file(&self, tab: &str) -> PathBuf {
        self.paths.notes_file(Path::new(REPO), tab)
    }
}

fn rpc(id: u64, method: &str, params: &Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

fn call(id: u64, tool: &str, args: &Value) -> String {
    rpc(id, "tools/call", &json!({"name": tool, "arguments": args}))
}

#[test]
fn the_url_and_token_match_what_launch_expects() {
    let h = Harness::start();
    let hook = h.register(1, "a");
    assert!(hook.base_url.starts_with("http://127.0.0.1:"), "{}", hook.base_url);
    assert!(!hook.base_url.ends_with('/'));
    assert_eq!(hook.token.len(), 32);
    assert_eq!(hook.hook_url("claude"), format!("{}/hook/claude", hook.base_url));
    assert_eq!(hook.mcp_url(), format!("{}/mcp", hook.base_url));
    assert_ne!(h.register(2, "b").token, hook.token);
}

#[test]
fn requests_without_a_valid_token_get_401() {
    let h = Harness::start();
    let hook = h.register(1, "a");
    for token in [None, Some("wrong"), Some("")] {
        for (method, path, body) in [
            ("POST", "/hook/claude", "{}"),
            ("POST", "/mcp", "{}"),
            ("GET", "/mcp", ""),
            ("GET", "/x", ""),
        ] {
            let reply = h.client().request(method, path, token, body);
            assert_eq!(reply.status, 401, "{method} {path} {token:?}");
            assert_eq!(reply.header("www-authenticate"), Some("Bearer"));
        }
    }
    let mut fresh = h.client();
    assert_eq!(fresh.request("POST", "/hook/claude", Some(&hook.token), "{}").status, 200);
    h.no_more_events();
}

#[test]
fn claude_hook_payloads_arrive_as_events_with_a_repaint() {
    let h = Harness::start();
    let hook = h.register(7, "a");
    let mut client = h.client();
    let stop = json!({"session_id": "s", "hook_event_name": "Stop", "last_assistant_message": "hi",
        "transcript_path": "/t/s.jsonl"})
    .to_string();
    let reply = client.request("POST", "/hook/claude", Some(&hook.token), &stop);
    assert_eq!((reply.status, reply.body.as_str()), (200, "{}"));
    assert_eq!(reply.header("content-type"), Some("application/json"));
    let message = h.next_event();
    assert_eq!(message.tab_id, 7);
    let TabEvent::Hook(event) = message.event else { panic!("not a hook event") };
    assert_eq!(event.kind, HookKind::TurnDone);
    assert_eq!(event.message.as_deref(), Some("hi"));
    assert_eq!(event.transcript_path, Some(PathBuf::from("/t/s.jsonl")));
    assert_eq!(h.repaints.load(Ordering::SeqCst), 1);
}

#[test]
fn a_curl_style_hook_post_with_expect_continue_works() {
    let h = Harness::start();
    let hook = h.register(1, "a");
    let mut client = h.client();
    let body = json!({"hook_event_name": "AfterAgent", "prompt_response": "ok"}).to_string();
    let head = format!(
        "POST /hook/gemini HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nExpect: 100-continue\r\nContent-Length: {}\r\n\r\n",
        hook.token,
        body.len()
    );
    client.send_raw(head.as_bytes());
    let interim = client.read_reply().expect("100 Continue");
    assert_eq!(interim.status, 100);
    client.send_raw(body.as_bytes());
    assert_eq!(client.read_reply().unwrap().status, 200);
    let TabEvent::Hook(event) = h.next_event().event else { panic!("not a hook event") };
    assert_eq!(event.kind, HookKind::TurnDone);
}

#[test]
fn mcp_over_http_runs_the_handshake_and_both_event_tools() {
    let h = Harness::start();
    let hook = h.register(3, "a");
    let token = Some(hook.token.as_str());
    let mut client = h.client();

    let init = client.request(
        "POST",
        "/mcp",
        token,
        &rpc(1, "initialize", &json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}})),
    );
    assert_eq!(init.status, 200);
    assert_eq!(init.json()["result"]["serverInfo"]["name"], "tidewatch");

    let note = client.request(
        "POST",
        "/mcp",
        token,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );
    assert_eq!((note.status, note.body.as_str()), (202, ""));

    let list = client.request("POST", "/mcp", token, &rpc(2, "tools/list", &json!({})));
    assert_eq!(list.json()["result"]["tools"].as_array().unwrap().len(), 4);

    let done =
        client.request("POST", "/mcp", token, &call(3, "stage_done", &json!({"summary": "hello"})));
    assert_eq!(done.json()["result"]["isError"], false);
    let finding = client.request(
        "POST",
        "/mcp",
        token,
        &call(
            4,
            "report_finding",
            &json!({"file": "a.rs", "line": 9, "blocking": true, "text": "bug"}),
        ),
    );
    assert_eq!(finding.status, 200);

    assert_eq!(
        h.next_event(),
        TabMessage {
            tab_id: 3,
            generation: 0,
            event: TabEvent::StageDone { summary: "hello".into() }
        }
    );
    assert_eq!(
        h.next_event(),
        TabMessage {
            tab_id: 3,
            generation: 0,
            event: TabEvent::Finding {
                file: "a.rs".into(),
                line: 9,
                blocking: true,
                text: "bug".into()
            },
        }
    );

    let get = client.request("GET", "/mcp", token, "");
    assert_eq!(get.status, 405);
    assert_eq!(get.header("allow"), Some("POST"));

    let bad = client.request("POST", "/mcp", token, "{{{");
    assert_eq!(bad.json()["error"]["code"], -32700);
    let unknown = client.request("POST", "/mcp", token, &call(5, "nope", &json!({})));
    assert_eq!(unknown.json()["error"]["code"], -32602);
    h.no_more_events();
}

#[test]
fn notes_round_trip_over_http_through_the_temp_home() {
    use kelp_core::review::{Anchor, Comment, Side};
    use tidewatch_core::notes::Notes;
    let h = Harness::start();
    let hook = h.register(1, "tab-a");
    let file = h.notes_file("tab-a");
    let mut notes = Notes::default();
    let anchor =
        Anchor { side: Side::New, line_hint: 4, text: "x".into(), before: vec![], after: vec![] };
    let id = notes.add(
        "src/a.rs",
        None,
        anchor,
        Comment { author: "You".into(), time: 5, body: "rename".into() },
    );
    notes.save(&file).unwrap();

    let mut client = h.client();
    let token = Some(hook.token.as_str());
    let listed = client.request("POST", "/mcp", token, &call(1, "list_notes", &json!({})));
    let text = listed.json()["result"]["content"][0]["text"].as_str().unwrap().to_owned();
    let threads: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(threads[0]["comments"][0]["body"], "rename");

    let reply = client.request(
        "POST",
        "/mcp",
        token,
        &call(2, "reply_to_note", &json!({"id": id, "text": "renamed"})),
    );
    assert_eq!(reply.json()["result"]["isError"], false);
    assert_eq!(
        h.next_event(),
        TabMessage {
            tab_id: 1,
            generation: 0,
            event: TabEvent::NoteReply { note_id: id, text: "renamed".into() }
        }
    );
    let saved = Notes::load(&file);
    assert_eq!(saved.threads[0].comments[1].author, "claude");
    assert_eq!(saved.threads[0].comments[1].body, "renamed");

    let missing = client.request(
        "POST",
        "/mcp",
        token,
        &call(3, "reply_to_note", &json!({"id": 99, "text": "x"})),
    );
    assert_eq!(missing.json()["error"]["code"], -32602);
}

#[test]
fn unregistering_a_tab_revokes_its_token_and_registering_again_rotates_it() {
    let h = Harness::start();
    let first = h.register(1, "a");
    let status = |token: &str| h.client().request("POST", "/hook/claude", Some(token), "{}").status;
    assert_eq!(status(&first.token), 200);
    let second = h.register(1, "a");
    assert_ne!(first.token, second.token);
    assert_eq!(status(&first.token), 401);
    assert_eq!(status(&second.token), 200);
    h.listener.unregister_tab(1);
    assert_eq!(status(&second.token), 401);
}

#[test]
fn messages_carry_the_generation_their_token_registered_with() {
    let h = Harness::start();
    let stage_one = h.register_generation(5, "a", 1);
    let done = |token: &str, summary: &str| {
        h.client().request(
            "POST",
            "/mcp",
            Some(token),
            &call(1, "stage_done", &json!({ "summary": summary })),
        )
    };
    assert_eq!(done(&stage_one.token, "first").status, 200);
    let stage_two = h.register_generation(5, "a", 2);
    assert_eq!(done(&stage_one.token, "late").status, 401);
    assert_eq!(done(&stage_two.token, "second").status, 200);
    let seen: Vec<(u64, TabEvent)> = std::iter::repeat_with(|| h.next_event())
        .take(2)
        .map(|m| (m.generation, m.event))
        .collect();
    assert_eq!(
        seen,
        [
            (1, TabEvent::StageDone { summary: "first".into() }),
            (2, TabEvent::StageDone { summary: "second".into() }),
        ]
    );
    h.no_more_events();
}

#[test]
fn one_connection_serves_many_requests_and_a_close_header_ends_it() {
    let h = Harness::start();
    let hook = h.register(1, "a");
    let mut client = h.client();
    for n in 0..20 {
        let reply = client.request("POST", "/mcp", Some(&hook.token), &rpc(n, "ping", &json!({})));
        assert_eq!(reply.json()["id"], n);
        assert_eq!(reply.header("connection"), Some("keep-alive"));
    }
    let text = format!(
        "POST /mcp HTTP/1.1\r\nAuthorization: Bearer {}\r\nConnection: close\r\nContent-Length: 2\r\n\r\n{{}}",
        hook.token
    );
    client.send_raw(text.as_bytes());
    let last = client.read_reply().unwrap();
    assert_eq!(last.header("connection"), Some("close"));
    assert!(client.read_reply().is_none(), "the server closed the connection");
}

#[test]
fn pipelined_requests_in_one_write_are_answered_in_order() {
    let h = Harness::start();
    let hook = h.register(1, "a");
    let mut client = h.client();
    let one = |id: u64| {
        let body = rpc(id, "ping", &json!({}));
        format!(
            "POST /mcp HTTP/1.1\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\n\r\n{body}",
            hook.token,
            body.len()
        )
    };
    client.send_raw(format!("{}{}{}", one(1), one(2), one(3)).as_bytes());
    let ids: Vec<Value> =
        (0..3).map(|_| client.read_reply().unwrap().json()["id"].clone()).collect();
    assert_eq!(ids, [json!(1), json!(2), json!(3)]);
}

#[test]
fn a_request_split_into_tiny_writes_still_parses() {
    let h = Harness::start();
    let hook = h.register(1, "a");
    let mut client = h.client();
    let body = rpc(1, "ping", &json!({}));
    let text = format!(
        "POST /mcp HTTP/1.1\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\n\r\n{body}",
        hook.token,
        body.len()
    );
    for piece in text.as_bytes().chunks(7) {
        client.send_raw(piece);
        client.stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(client.read_reply().unwrap().json()["result"], json!({}));
}

#[test]
fn chunked_request_bodies_are_decoded() {
    let h = Harness::start();
    let hook = h.register(1, "a");
    let mut client = h.client();
    let body = call(1, "stage_done", &json!({"summary": "chunked hello"}));
    let (first, second) = body.split_at(body.len() / 2);
    let text = format!(
        "POST /mcp HTTP/1.1\r\nAuthorization: Bearer {}\r\nTransfer-Encoding: chunked\r\n\r\n{:x};ext=1\r\n{first}\r\n{:X}\r\n{second}\r\n0\r\nTrailer: x\r\n\r\n",
        hook.token,
        first.len(),
        second.len()
    );
    client.send_raw(text.as_bytes());
    assert_eq!(client.read_reply().unwrap().json()["result"]["isError"], false);
    assert_eq!(h.next_event().event, TabEvent::StageDone { summary: "chunked hello".into() });
    let bad = format!(
        "POST /mcp HTTP/1.1\r\nAuthorization: Bearer {}\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n",
        hook.token
    );
    let mut second_client = h.client();
    second_client.send_raw(bad.as_bytes());
    assert_eq!(second_client.read_reply().unwrap().status, 400);
}

#[test]
fn malformed_and_oversized_requests_are_refused_without_killing_the_server() {
    let h = Harness::start();
    let hook = h.register(1, "a");

    let mut garbage = h.client();
    garbage.send_raw(b"this is not http\r\n\r\n");
    assert_eq!(garbage.read_reply().unwrap().status, 400);

    let mut huge_head = h.client();
    huge_head
        .send_raw(format!("POST /mcp HTTP/1.1\r\nX-Pad: {}\r\n", "a".repeat(20_000)).as_bytes());
    assert_eq!(huge_head.read_reply().unwrap().status, 431);

    let mut huge_body = h.client();
    huge_body.send_raw(
        format!(
            "POST /mcp HTTP/1.1\r\nAuthorization: Bearer {}\r\nContent-Length: 999999999\r\n\r\n",
            hook.token
        )
        .as_bytes(),
    );
    assert_eq!(huge_body.read_reply().unwrap().status, 413);

    let mut unauthorised_huge = h.client();
    unauthorised_huge.send_raw(b"POST /mcp HTTP/1.1\r\nContent-Length: 999999999\r\n\r\n");
    assert_eq!(unauthorised_huge.read_reply().unwrap().status, 401);

    let mut bad_length = h.client();
    bad_length.send_raw(
        format!(
            "POST /mcp HTTP/1.1\r\nAuthorization: Bearer {}\r\nContent-Length: -1\r\n\r\n",
            hook.token
        )
        .as_bytes(),
    );
    assert_eq!(bad_length.read_reply().unwrap().status, 400);

    let mut abrupt = h.client();
    abrupt.send_raw(
        format!(
            "POST /mcp HTTP/1.1\r\nAuthorization: Bearer {}\r\nContent-Length: 50\r\n\r\nshort",
            hook.token
        )
        .as_bytes(),
    );
    drop(abrupt);

    let mut healthy = h.client();
    assert_eq!(
        healthy.request("POST", "/mcp", Some(&hook.token), &rpc(1, "ping", &json!({}))).status,
        200
    );
    h.no_more_events();
}

#[test]
fn many_tabs_talking_at_once_keep_their_events_apart() {
    let h = Harness::start();
    let addr = h.addr();
    let hooks: Vec<(u64, HookListener)> =
        (0..8).map(|n| (n, h.register(n, &format!("tab-{n}")))).collect();
    std::thread::scope(|scope| {
        for (tab_id, hook) in &hooks {
            scope.spawn(move || {
                let mut client = Client::connect(addr);
                for round in 0..10 {
                    let summary = format!("tab {tab_id} round {round}");
                    let reply = client.request(
                        "POST",
                        "/mcp",
                        Some(&hook.token),
                        &call(round, "stage_done", &json!({"summary": summary})),
                    );
                    assert_eq!(reply.status, 200);
                }
            });
        }
    });
    let mut per_tab = std::collections::BTreeMap::<u64, Vec<String>>::new();
    for _ in 0..80 {
        let message = h.next_event();
        let TabEvent::StageDone { summary } = message.event else { panic!("unexpected event") };
        per_tab.entry(message.tab_id).or_default().push(summary);
    }
    assert_eq!(per_tab.len(), 8);
    for (tab_id, summaries) in per_tab {
        let want: Vec<String> =
            (0..10).map(|round| format!("tab {tab_id} round {round}")).collect();
        assert_eq!(summaries, want, "tab {tab_id}");
    }
    assert_eq!(h.repaints.load(Ordering::SeqCst), 80);
}

#[test]
fn dropping_the_listener_stops_it_from_accepting() {
    let h = Harness::start();
    let addr = h.addr();
    assert!(TcpStream::connect(addr).is_ok());
    drop(h);
    assert!(TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_err());
}
