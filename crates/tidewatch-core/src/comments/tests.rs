use super::*;

fn kinds(text: &str, syntax: Syntax) -> Vec<Kind> {
    scan(text, syntax, 1).iter().map(|flag| flag.kind).collect()
}

#[test]
fn a_path_picks_its_syntax_and_generated_or_vendored_code_is_left_alone() {
    let cases = [
        ("src/main.rs", Some(Syntax::CLike)),
        ("web/app.tsx", Some(Syntax::CLike)),
        ("lib/tool.rb", Some(Syntax::Hash)),
        ("scripts/run.sh", Some(Syntax::Hash)),
        ("tools/x.py", Some(Syntax::Hash)),
        ("db/query.sql", Some(Syntax::Sql)),
        ("README.md", None),
        ("config.toml", None),
        ("node_modules/pkg/index.js", None),
        ("vendor/lib/x.rb", None),
        ("web/dist/app.js", None),
        ("types/api.d.ts", None),
        ("app.min.js", None),
        ("spec/__snapshots__/a.snap", None),
        ("test/fixtures/sample.rb", None),
        ("db/migrate/001_x.rb", None),
        ("target/debug/build.rs", None),
        ("Makefile", None),
    ];
    for (path, want) in cases {
        assert_eq!(Syntax::of_path(path), want, "{path}");
    }
}

#[test]
fn rust_comments_are_kept_or_flagged() {
    let cases: [(&str, Vec<Kind>); 16] = [
        ("// Copyright 2026 Acme\nfn a() {}", vec![]),
        ("// SPDX-License-Identifier: MIT\nfn a() {}", vec![]),
        ("//! Parses the thing the way the vendor's protocol forces.\nfn a() {}", vec![]),
        ("// SAFETY: the pointer outlives the call\nfn a() {}", vec![]),
        ("// see https://example.com/rfc for the quirk\nfn a() {}", vec![]),
        ("// workaround for rust-lang/rust#12345\nfn a() {}", vec![]),
        ("// loop over the items\nfor x in xs {}", vec![Kind::Narration]),
        ("// ===========\nfn a() {}", vec![Kind::Banner]),
        ("// ---- helpers ----\nfn a() {}", vec![Kind::Banner]),
        ("// let x = foo(1);\nfn a() {}", vec![Kind::DeadCode]),
        ("// fn old() {}\nfn a() {}", vec![Kind::DeadCode]),
        ("// IMPORTANT do not touch\nfn a() {}", vec![Kind::Alibi]),
        ("// hack for now\nfn a() {}", vec![Kind::Alibi]),
        ("// TODO: handle errors\nfn a() {}", vec![Kind::BareTodo]),
        ("// TODO(#123): handle errors\nfn a() {}", vec![]),
        ("let x = 1; // the count\n", vec![]),
    ];
    for (text, want) in cases {
        assert_eq!(kinds(text, Syntax::CLike), want, "{text:?}");
    }
}

#[test]
fn rust_attributes_and_derive_lines_are_code_not_comments() {
    let text = "#[derive(Debug)]\nstruct A;\n#[allow(clippy::x)]\nfn b() {}\n";
    assert_eq!(kinds(text, Syntax::CLike), vec![]);
}

#[test]
fn a_longer_explanation_that_starts_with_a_verb_is_a_why_not_narration() {
    let why = "// Handle the case where the vendor returns a cursor that points past the final page\nfn a() {}";
    assert_eq!(kinds(why, Syntax::CLike), vec![]);
    assert_eq!(kinds("// Handle errors\nfn a() {}", Syntax::CLike), vec![Kind::Narration]);
    assert_eq!(kinds("// Computes the total\nfn a() {}", Syntax::CLike), vec![Kind::Narration]);
}

#[test]
fn trailing_comments_are_judged_and_urls_and_strings_are_not_comments() {
    let cases = [
        ("let n = 0; // set n to zero", vec![Kind::Narration]),
        ("let n = 0; // hack", vec![Kind::Alibi]),
        ("let url = \"http://example.com\";", vec![]),
        ("let s = \"a // b\";", vec![]),
        ("let n = 0; // forced by the vendor's off-by-one cursor", vec![]),
    ];
    for (text, want) in cases {
        assert_eq!(kinds(text, Syntax::CLike), want, "{text:?}");
    }
}

#[test]
fn doc_comments_are_kept_unless_they_restate_the_signature_or_are_alibis() {
    let cases: [(&str, Vec<Kind>); 9] = [
        ("/// Returns the name.\npub fn name(&self) -> &str {}", vec![Kind::Restates]),
        ("/// Gets the user id.\npub fn user_id(&self) -> u64 {}", vec![Kind::Restates]),
        (
            "/// The name shown in the tab strip, trimmed to 40 characters.\npub fn name(&self) -> &str {}",
            vec![],
        ),
        (
            "/// Refuses a branch that is not the tab's own.\npub fn check_branch(b: &str) {}",
            vec![],
        ),
        ("/// Opens the file.\n#[must_use]\npub fn open_file() {}", vec![Kind::Restates]),
        ("/// IMPORTANT this must stay\npub fn a() {}", vec![Kind::Alibi]),
        ("/// Example:\n/// let x = foo(1);\npub fn a() {}", vec![]),
        ("//! Module header that says what the file is for.\n\nuse std::fmt;", vec![]),
        ("/// ======\npub fn a() {}", vec![Kind::Banner]),
    ];
    for (text, want) in cases {
        assert_eq!(kinds(text, Syntax::CLike), want, "{text:?}");
    }
}

#[test]
fn typescript_and_javascript_comments() {
    let cases: [(&str, Vec<Kind>); 14] = [
        ("// @ts-check\nconst a = 1;", vec![]),
        ("/* prettier-ignore */\nconst a = 1;", vec![]),
        ("// eslint-disable-next-line no-console\nconsole.log(1);", vec![Kind::Suppression]),
        (
            "// eslint-disable-next-line no-console -- the vendor SDK needs it\nconsole.log(1);",
            vec![],
        ),
        ("// @ts-expect-error vendor typing misses the overload\nfoo();", vec![]),
        ("// @ts-ignore\nfoo();", vec![Kind::Suppression]),
        ("// console.log(value);\nconst a = 1;", vec![Kind::DeadCode]),
        ("// const old = () => 1;\nconst a = 1;", vec![Kind::DeadCode]),
        ("/**\n * Returns the user.\n */\nexport function getUser() {}", vec![Kind::Restates]),
        (
            "/** The signed-in user, or null before the session loads. */\nexport function getUser() {}",
            vec![],
        ),
        ("/* increment the counter */\ncounter++;", vec![Kind::Narration]),
        ("// ******************\nconst a = 1;", vec![Kind::Banner]),
        ("// FIXME: later\nconst a = 1;", vec![Kind::Alibi]),
        ("const a = 1; // eslint-disable-line", vec![Kind::Suppression]),
    ];
    for (text, want) in cases {
        assert_eq!(kinds(text, Syntax::CLike), want, "{text:?}");
    }
}

#[test]
fn block_comments_span_lines_and_are_judged_per_line() {
    let text = "/*\n * Copyright 2026 Acme\n * Licensed under the MIT license\n */\nconst a = 1;";
    assert_eq!(kinds(text, Syntax::CLike), vec![]);
    let noisy = "/*\n * loop over the items\n */\nconst a = 1;";
    assert_eq!(kinds(noisy, Syntax::CLike), vec![Kind::Narration]);
}

#[test]
fn python_comments() {
    let cases: [(&str, Vec<Kind>); 9] = [
        ("#!/usr/bin/env python3\nprint(1)", vec![]),
        ("# -*- coding: utf-8 -*-\nx = 1", vec![]),
        ("# type: ignore\nx = 1", vec![Kind::Suppression]),
        ("x = foo()  # noqa: E501 the vendor line is long", vec![]),
        ("x = foo()  # noqa", vec![Kind::Suppression]),
        ("# initialize the list\nxs = []", vec![Kind::Narration]),
        ("# xs.append(1)\nxs = []", vec![Kind::DeadCode]),
        ("# TODO fix\nxs = []", vec![Kind::BareTodo]),
        ("# ==================\nxs = []", vec![Kind::Banner]),
    ];
    for (text, want) in cases {
        assert_eq!(kinds(text, Syntax::Hash), want, "{text:?}");
    }
}

#[test]
fn ruby_comments() {
    let cases: [(&str, Vec<Kind>); 6] = [
        ("# frozen_string_literal: true\n\nclass A; end", vec![]),
        ("# typed: strict\nclass A; end", vec![]),
        ("# rubocop:disable Metrics/MethodLength\ndef a; end", vec![Kind::Suppression]),
        ("# rubocop:disable Metrics/MethodLength - one long table\ndef a; end", vec![]),
        ("# set up the user\nuser = User.new", vec![Kind::Narration]),
        ("# user.save!\nuser = User.new", vec![Kind::DeadCode]),
    ];
    for (text, want) in cases {
        assert_eq!(kinds(text, Syntax::Hash), want, "{text:?}");
    }
}

#[test]
fn shell_comments() {
    let cases: [(&str, Vec<Kind>); 6] = [
        ("#!/bin/sh\nset -e", vec![]),
        (
            "#!/usr/bin/env bash\n# shellcheck disable=SC2086 because the list is word split on purpose\nrm $x",
            vec![],
        ),
        ("# loop over files\nfor f in *; do echo $f; done", vec![Kind::Narration]),
        ("echo \"${#x}\"", vec![]),
        ("echo $#", vec![]),
        ("# ------------\nset -e", vec![Kind::Banner]),
    ];
    for (text, want) in cases {
        assert_eq!(kinds(text, Syntax::Hash), want, "{text:?}");
    }
}

#[test]
fn sql_comments() {
    assert_eq!(
        kinds("-- fetch the users\nselect * from users;", Syntax::Sql),
        vec![Kind::Narration]
    );
    assert_eq!(kinds("-- see https://example.com/why\nselect 1;", Syntax::Sql), vec![]);
}

#[test]
fn five_comment_lines_in_a_row_are_a_bloat_and_docs_are_exempt() {
    let block = "// one\n// two\n// three\n// four\n// five\nfn a() {}";
    assert!(kinds(block, Syntax::CLike).contains(&Kind::DocBloat));
    let docs = "/// one\n/// two\n/// three\n/// four\n/// five\n/// six\npub fn a() {}";
    assert!(!kinds(docs, Syntax::CLike).contains(&Kind::DocBloat));
}

#[test]
fn many_comments_in_little_code_are_dense_but_a_few_are_not() {
    let dense = "// a\nlet a = 1;\n// b\nlet b = 1;\n// c\nlet c = 1;\n// d\nlet d = 1;\n// e\nlet e = 1;\n";
    assert!(kinds(dense, Syntax::CLike).contains(&Kind::Density));
    let sparse = "// forced by the vendor protocol, see the spec\nlet a = 1;\nlet b = 1;\nlet c = 1;\nlet d = 1;\nlet e = 1;\n";
    assert!(!kinds(sparse, Syntax::CLike).contains(&Kind::Density));
}

#[test]
fn line_numbers_count_from_the_first_line_given() {
    let flags = scan("let a = 1;\n// loop over a\nlet b = 2;", Syntax::CLike, 40);
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].line, 41);
    assert_eq!(flags[0].text, "loop over a");
}

#[test]
fn a_long_flagged_comment_is_clipped_in_the_report() {
    let long = format!("// TODO {}\nfn a() {{}}", "word ".repeat(40));
    let flags = scan(&long, Syntax::CLike, 1);
    assert!(flags[0].text.ends_with("..."));
    assert!(flags[0].text.chars().count() <= 93);
}

#[test]
fn the_report_names_the_place_the_kind_and_the_text() {
    let flags = scan("let a = 1;\n// loop over a", Syntax::CLike, 1);
    assert_eq!(
        render(&flags),
        ["L2  [narration] narrates what the next line already says: loop over a"]
    );
    let dense = Flag { line: 0, kind: Kind::Density, text: "6 comment lines in 20 lines".into() };
    assert_eq!(
        render(&[dense]),
        ["file  [density] too many comments for the code written: 6 comment lines in 20 lines"]
    );
}

#[test]
fn clean_code_and_empty_text_have_no_flags() {
    assert_eq!(kinds("", Syntax::CLike), vec![]);
    assert_eq!(kinds("fn a() {}\n\nfn b() {}\n", Syntax::CLike), vec![]);
}
