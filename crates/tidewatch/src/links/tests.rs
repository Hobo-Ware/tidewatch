use super::*;

fn found(text: &str) -> Vec<(String, String)> {
    let plain = plain(text);
    plain.links.iter().map(|l| (plain.text[l.range.clone()].to_owned(), l.url.clone())).collect()
}

fn pair(shown: &str, url: &str) -> (String, String) {
    (shown.to_owned(), url.to_owned())
}

#[test]
fn plain_text_finds_web_links() {
    let cases: [(&str, Vec<(&str, &str)>); 24] = [
        (
            "see http://localhost:8787/jobs now",
            vec![("http://localhost:8787/jobs", "http://localhost:8787/jobs")],
        ),
        (
            "https://github.com/owner/repo/pull/12",
            vec![(
                "https://github.com/owner/repo/pull/12",
                "https://github.com/owner/repo/pull/12",
            )],
        ),
        ("open localhost:5174", vec![("localhost:5174", "http://localhost:5174")]),
        (
            "open 127.0.0.1:8787/jobs?x=1.",
            vec![("127.0.0.1:8787/jobs?x=1", "http://127.0.0.1:8787/jobs?x=1")],
        ),
        ("at localhost:5174, then", vec![("localhost:5174", "http://localhost:5174")]),
        ("(https://example.com/a)", vec![("https://example.com/a", "https://example.com/a")]),
        ("(see https://example.com/a).", vec![("https://example.com/a", "https://example.com/a")]),
        ("[https://example.com/a]", vec![("https://example.com/a", "https://example.com/a")]),
        (
            "it is 'https://example.com/a'.",
            vec![("https://example.com/a", "https://example.com/a")],
        ),
        (
            "say \"https://example.com/a\";",
            vec![("https://example.com/a", "https://example.com/a")],
        ),
        ("at https://example.com/a: yes", vec![("https://example.com/a", "https://example.com/a")]),
        ("`https://example.com/a`", vec![("https://example.com/a", "https://example.com/a")]),
        ("`localhost:5174`", vec![("localhost:5174", "http://localhost:5174")]),
        ("**https://example.com/a**", vec![("https://example.com/a", "https://example.com/a")]),
        (
            "https://en.wikipedia.org/wiki/Foo_(bar)",
            vec![(
                "https://en.wikipedia.org/wiki/Foo_(bar)",
                "https://en.wikipedia.org/wiki/Foo_(bar)",
            )],
        ),
        ("HTTPS://EXAMPLE.com", vec![("HTTPS://EXAMPLE.com", "HTTPS://EXAMPLE.com")]),
        (
            "[the PR](https://github.com/o/r/pull/1) landed",
            vec![("the PR", "https://github.com/o/r/pull/1")],
        ),
        ("[docs](https://x.dev/a_(b) \"title\")", vec![("docs", "https://x.dev/a_(b)")]),
        ("[the app](localhost:5174)", vec![("the app", "http://localhost:5174")]),
        (
            "a https://a.com and localhost:3000/x b",
            vec![
                ("https://a.com", "https://a.com"),
                ("localhost:3000/x", "http://localhost:3000/x"),
            ],
        ),
        ("[a [b]](https://x.dev)", vec![("a [b]", "https://x.dev")]),
        (
            "https://claude.ai/artifact/abc-123",
            vec![("https://claude.ai/artifact/abc-123", "https://claude.ai/artifact/abc-123")],
        ),
        (
            "https://example.com/a?b=1&c=2#frag",
            vec![("https://example.com/a?b=1&c=2#frag", "https://example.com/a?b=1&c=2#frag")],
        ),
        ("(localhost:5174)", vec![("localhost:5174", "http://localhost:5174")]),
    ];
    for (text, want) in cases {
        let want: Vec<_> = want.into_iter().map(|(s, u)| pair(s, u)).collect();
        assert_eq!(found(text), want, "text {text:?}");
    }
}

#[test]
fn plain_text_has_no_false_positives() {
    let cases = [
        "file.rs:12",
        "src/app.rs:120:4",
        "a:b",
        "at 16:02 today",
        "PR #12",
        "mail me@example.com",
        "mail me@localhost:5432",
        "see www.example.com",
        "xhttps://example.com",
        "foo.localhost:3000",
        "localhost:",
        "localhost:abc",
        "localhost:99999",
        "localhost:5174abc",
        "https://",
        "https:// nothing",
        "ftp://example.com/a",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "[x](javascript:alert(1))",
        "[x](file:///etc/passwd)",
        "[x](/relative/path)",
        "[x]()",
        "see [1] and (2)",
    ];
    for text in cases {
        assert_eq!(found(text), vec![], "text {text:?}");
    }
}

#[test]
fn plain_text_shows_the_label_of_a_markdown_link() {
    let cases = [
        ("[the PR](https://x.dev/1) landed", "the PR landed"),
        ("[bad](javascript:alert(1)) link", "bad link"),
        ("go to https://x.dev now", "go to https://x.dev now"),
    ];
    for (text, shown) in cases {
        assert_eq!(plain(text).text, shown, "text {text:?}");
    }
}

#[test]
fn only_http_and_https_are_web_urls() {
    let cases = [
        ("http://localhost:5174", true),
        ("https://x.dev/a b", false),
        ("https://x.dev/a", true),
        ("HTTP://X.dev", true),
        ("https://", false),
        ("http:// x", false),
        ("ftp://x.dev", false),
        ("file:///x", false),
        ("javascript:alert(1)", false),
        ("data:text/html,x", false),
        ("localhost:5174", false),
        ("https://x.dev/\u{0}", false),
    ];
    for (url, want) in cases {
        assert_eq!(is_web_url(url), want, "url {url:?}");
    }
}

#[test]
fn markdown_rewrites_every_web_link_as_a_markdown_link() {
    let cases = [
        ("no links here", "no links here"),
        ("open localhost:5174 now", "open [localhost:5174](http://localhost:5174) now"),
        ("see https://x.dev/a.", "see [https://x.dev/a](https://x.dev/a)."),
        ("(https://x.dev/a)", "([https://x.dev/a](https://x.dev/a))"),
        (
            "run `http://localhost:8787/jobs` first",
            "run [`http://localhost:8787/jobs`](http://localhost:8787/jobs) first",
        ),
        ("run `curl http://localhost:8787` first", "run `curl http://localhost:8787` first"),
        ("run ``localhost:3000`` ok", "run [``localhost:3000``](http://localhost:3000) ok"),
        ("[the PR](https://x.dev/1)", "[the PR](https://x.dev/1)"),
        ("[bad](javascript:alert(1)) link", "bad link"),
        ("[bad](file:///etc/passwd)", "bad"),
        ("<https://x.dev/a>", "[https://x.dev/a](https://x.dev/a)"),
        ("<javascript:alert(1)>", "\\<javascript:alert(1)>"),
        ("<me@example.com>", "\\<me@example.com>"),
        ("<div>", "<div>"),
        ("https://x.dev/a_b_c*d", "[https://x.dev/a\\_b\\_c\\*d](https://x.dev/a_b_c*d)"),
        ("https://x.dev/a(b)", "[https://x.dev/a(b)](https://x.dev/a\\(b\\))"),
        ("wow!https://x.dev", "wow\\![https://x.dev](https://x.dev)"),
        ("![img](https://x.dev/a.png)", "![img](https://x.dev/a.png)"),
        ("[1]: https://x.dev/a", "[1]: https://x.dev/a"),
        ("file.rs:12 and a:b at 16:02", "file.rs:12 and a:b at 16:02"),
        (
            "```\nhttps://x.dev/a\n```\nthen https://x.dev/b\n",
            "```\nhttps://x.dev/a\n```\nthen [https://x.dev/b](https://x.dev/b)\n",
        ),
        ("~~~sh\nlocalhost:5174\n~~~", "~~~sh\nlocalhost:5174\n~~~"),
        ("```\nunclosed https://x.dev/a", "```\nunclosed https://x.dev/a"),
        (
            "a https://x.dev/a\nb https://x.dev/b",
            "a [https://x.dev/a](https://x.dev/a)\nb [https://x.dev/b](https://x.dev/b)",
        ),
    ];
    for (md, want) in cases {
        assert_eq!(markdown(md, None), want, "markdown {md:?}");
    }
}

#[test]
fn a_bare_number_links_to_the_repos_issue_only_when_the_repo_is_known() {
    let repo = Some("acme/app");
    let link = |n: &str| format!("[#{n}](https://github.com/acme/app/issues/{n})");
    let cases = [
        ("see #2900", format!("see {}", link("2900"))),
        ("PR #12 and (#7).", format!("PR {} and ({}).", link("12"), link("7"))),
        ("#5 first", format!("{} first", link("5"))),
        ("## Fixed #3", format!("## Fixed {}", link("3"))),
        ("color #fff", "color #fff".to_owned()),
        ("issue#12", "issue#12".to_owned()),
        ("x/a/b#12 http://a/b#12", "x/a/b#12 [http://a/b#12](http://a/b#12)".to_owned()),
        ("&#12; ##12 #012 #12a #12345678", "&#12; ##12 #012 #12a #12345678".to_owned()),
        ("`#12` and [x #12](https://x.dev)", "`#12` and [x #12](https://x.dev)".to_owned()),
        ("```\n#12\n```", "```\n#12\n```".to_owned()),
    ];
    for (md, want) in cases {
        assert_eq!(markdown(md, repo), want, "markdown {md:?}");
    }
    assert!(matches!(markdown("see #2900", None), Cow::Borrowed(_)));
    assert_eq!(plain("see #2900").links, Vec::new());
}

#[test]
fn a_number_after_another_repos_name_never_links_to_the_tabs_repo() {
    let repo = Some("acme/app");
    let ours = |n: &str| format!("[#{n}](https://github.com/acme/app/issues/{n})");
    let cases = [
        (
            "see Hobo-Ware/kelp#45.",
            "see [Hobo-Ware/kelp#45](https://github.com/Hobo-Ware/kelp/issues/45).".to_owned(),
        ),
        (
            "(other/lib.rs#3)",
            "([other/lib.rs#3](https://github.com/other/lib.rs/issues/3))".to_owned(),
        ),
        ("Hobo-Ware/kelp #45", "Hobo-Ware/kelp #45".to_owned()),
        ("Hobo-Ware/kelp PR #45", "Hobo-Ware/kelp PR #45".to_owned()),
        ("trakt/web (#317)", "trakt/web (#317)".to_owned()),
        ("`Hobo-Ware/kelp`: pull request #45", "`Hobo-Ware/kelp`: pull request #45".to_owned()),
        ("acme/app PR #12", format!("acme/app PR {}", ours("12"))),
        ("Acme/App #12", format!("Acme/App {}", ours("12"))),
        ("on feat/socials. PR #9", format!("on feat/socials. PR {}", ours("9"))),
        ("and/or #9", "and/or #9".to_owned()),
    ];
    for (md, want) in cases {
        assert_eq!(markdown(md, repo), want, "markdown {md:?}");
    }
}

#[test]
fn markdown_borrows_when_nothing_changes() {
    assert!(matches!(markdown("plain words, no links", None), Cow::Borrowed(_)));
    assert!(matches!(markdown("[ok](https://x.dev)", None), Cow::Borrowed(_)));
    assert!(matches!(markdown("see https://x.dev", None), Cow::Owned(_)));
}

#[test]
fn scanning_never_panics_on_odd_input() {
    let pieces = [
        "[",
        "]",
        "(",
        ")",
        "`",
        "<",
        ">",
        "!",
        "\\",
        "\n",
        "http://",
        "localhost:",
        "é",
        " ",
        "1",
        "\"",
        "'",
    ];
    for a in pieces {
        for b in pieces {
            for c in pieces {
                let text = format!("{a}{b}{c}x{a}{c}{b}");
                let _ = markdown(&text, None);
                let _ = plain(&text);
            }
        }
    }
}
