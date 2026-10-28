//! The port a service listens on, read from what it printed instead of asked for. A line that
//! says where it listens (`listening on`, `Local address`, `ready on`) beats a bare address, so
//! a proxy that logs each client's `127.0.0.1:50412` does not replace its own port. Pure.

const LOCAL_HOSTS: [&str; 4] = ["127.0.0.1", "localhost", "0.0.0.0", "[::1]"];
const LISTEN_WORDS: [&str; 8] = [
    "listening",
    "listen on",
    "local address",
    "serving on",
    "running on",
    "running at",
    "started on",
    "ready on",
];

/// The newest port the lines announce: the newest line that says where it listens, else the
/// newest line with a local address. A line with two addresses gives its last.
#[must_use]
pub fn newest<S: AsRef<str>>(lines: &[S]) -> Option<u16> {
    let announced = lines.iter().rev().find_map(|line| announced_port(line.as_ref()));
    announced.or_else(|| lines.iter().rev().find_map(|line| last_local_port(line.as_ref())))
}

fn announced_port(line: &str) -> Option<u16> {
    let lower = line.to_ascii_lowercase();
    let from = LISTEN_WORDS.iter().filter_map(|word| lower.find(word)).min()?;
    let rest = &line[from..];
    let colon = last_colon_port(rest);
    let word = port_after_word(&rest.to_ascii_lowercase());
    colon.max(word).map(|(_, port)| port).or_else(|| last_local_port(rest))
}

fn last_local_port(line: &str) -> Option<u16> {
    LOCAL_HOSTS
        .iter()
        .flat_map(|host| line.match_indices(host).map(move |(at, _)| (at, at + host.len())))
        .filter_map(|(at, end)| digits_after_colon(line, end).map(|port| (at, port)))
        .max_by_key(|(at, _)| *at)
        .map(|(_, port)| port)
}

fn last_colon_port(text: &str) -> Option<(usize, u16)> {
    text.match_indices(':')
        .filter_map(|(at, _)| digits_after_colon(text, at).map(|port| (at, port)))
        .next_back()
}

fn port_after_word(lower: &str) -> Option<(usize, u16)> {
    lower
        .match_indices("port ")
        .filter_map(|(at, word)| {
            let digits = leading_digits(&lower[at + word.len()..]);
            as_port(digits).map(|port| (at, port))
        })
        .last()
}

fn digits_after_colon(text: &str, colon_at: usize) -> Option<u16> {
    let rest = text.get(colon_at..)?.strip_prefix(':')?;
    let digits = leading_digits(rest);
    let after = rest[digits.len()..].chars().next();
    if after.is_some_and(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    as_port(digits)
}

fn leading_digits(text: &str) -> &str {
    let end = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    &text[..end]
}

fn as_port(digits: &str) -> Option<u16> {
    if digits.is_empty() || digits.len() > 5 {
        return None;
    }
    digits.parse::<u16>().ok().filter(|port| *port > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_port_a_line_announces() {
        let cases = [
            (
                "Local address to connect your application: 127.0.0.1:55926 (press ctrl-c to quit)",
                55926,
            ),
            ("Server listening on http://localhost:3000", 3000),
            ("  \u{279c}  Local:   http://localhost:5173/", 5173),
            ("ready on http://0.0.0.0:8080", 8080),
            ("listening on :9090", 9090),
            ("Listening on port 4000", 4000),
            ("listening on [::]:7000", 7000),
            ("Server listening on *:3001", 3001),
            ("proxy 127.0.0.1:1111 -> 127.0.0.1:2222", 2222),
            ("listening on 127.0.0.1:1111 and 127.0.0.1:2222", 2222),
            ("[12:30:45] listening on 127.0.0.1:3309", 3309),
        ];
        for (line, want) in cases {
            assert_eq!(newest(&[line]), Some(want), "{line}");
        }
    }

    #[test]
    fn lines_that_name_no_port_give_none() {
        let cases = [
            "",
            "ready",
            "12:30:45 started",
            "listening on 127.0.0.1:99999",
            "listening on 127.0.0.1:0",
            "see localhost:abc",
            "connect to 10.0.0.5:3306",
            "127.0.0.1:3000x",
            "listening somewhere",
        ];
        for line in cases {
            assert_eq!(newest(&[line]), None, "{line}");
        }
        assert_eq!(newest::<&str>(&[]), None);
    }

    #[test]
    fn the_newest_announcing_line_wins() {
        let lines = [
            "listening on 127.0.0.1:3000",
            "noise",
            "Local address to connect your application: 127.0.0.1:3309",
            "client 127.0.0.1:50412 connected",
            "client 127.0.0.1:50419 connected",
        ];
        assert_eq!(newest(&lines), Some(3309));
        let moved = ["listening on 127.0.0.1:3000", "listening on 127.0.0.1:3001"];
        assert_eq!(newest(&moved), Some(3001));
    }

    #[test]
    fn without_an_announcement_the_newest_local_address_is_used() {
        let lines = ["proxy up at 127.0.0.1:4100", "forwarding to localhost:4200", "idle"];
        assert_eq!(newest(&lines), Some(4200));
    }
}
