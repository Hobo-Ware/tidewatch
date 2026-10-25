use super::*;

fn plain(runs: &Inline) -> String {
    runs.iter().map(|run| run.text.as_str()).collect()
}

fn first_list(md: &str) -> (Option<u64>, Vec<Item>) {
    match parse(md).into_iter().next() {
        Some(Block::List { start, items }) => (start, items),
        other => panic!("not a list: {other:?}"),
    }
}

fn depth(items: &[Item]) -> usize {
    items
        .iter()
        .flat_map(|item| &item.blocks)
        .filter_map(|block| match block {
            Block::List { items, .. } => Some(1 + depth(items)),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

#[test]
fn the_owners_example_nests_a_sub_item_under_its_item() {
    let tree = parse("I am alist:\n- item\n    - sub-item\n");
    assert_eq!(tree.len(), 2);
    let Block::List { items, .. } = &tree[1] else { panic!("{tree:?}") };
    assert_eq!(items.len(), 1);
    let Block::List { items: inner, .. } = &items[0].blocks[1] else { panic!("{items:?}") };
    assert_eq!(inner.len(), 1);
    assert!(
        matches!(&inner[0].blocks[0], Block::Paragraph { runs, .. } if plain(runs) == "sub-item")
    );
}

#[test]
fn lists_nest_to_any_depth_with_two_four_space_and_tab_indents() {
    let cases = [
        ("- a\n  - b\n    - c\n      - d\n        - e", 4),
        ("- a\n    - b\n        - c\n            - d", 3),
        ("- a\n\t- b\n\t\t- c\n\t\t\t- d", 3),
        ("- a\n- b", 0),
        ("1. a\n   - b\n      1. c\n         - d", 3),
    ];
    for (input, want) in cases {
        let (_, items) = first_list(input);
        assert_eq!(depth(&items), want, "input {input:?}");
    }
}

#[test]
fn mixed_bullets_and_numbers_keep_their_kind_per_level() {
    let (start, items) = first_list("- a\n  1. b\n  2. c\n     - d");
    assert_eq!(start, None);
    let Block::List { start, items: inner } = &items[0].blocks[1] else { panic!() };
    assert_eq!((*start, inner.len()), (Some(1), 2));
    let Block::List { start, .. } = &inner[1].blocks[1] else { panic!() };
    assert_eq!(*start, None);
}

#[test]
fn a_numbered_list_keeps_its_start_number() {
    let cases = [("3. a\n4. b", 3), ("1. a", 1), ("10. a\n11. b", 10), ("0. a", 0)];
    for (input, want) in cases {
        assert_eq!(first_list(input).0, Some(want), "input {input:?}");
    }
}

#[test]
fn a_number_that_is_not_one_cannot_cut_into_a_paragraph() {
    let tree = parse("Numbered:\n3. third");
    assert_eq!(tree.len(), 1);
}

#[test]
fn tight_items_hold_tight_text_and_loose_items_hold_paragraphs() {
    let (_, tight) = first_list("- a\n- b");
    assert!(matches!(tight[0].blocks[0], Block::Paragraph { tight: true, .. }));
    let (_, loose) = first_list("- a\n\n- b");
    assert!(matches!(loose[0].blocks[0], Block::Paragraph { tight: false, .. }));
}

#[test]
fn an_item_holds_several_paragraphs_and_a_code_block() {
    let (_, items) = first_list("- one\n\n  two\n\n  ```rust\n  fn x() {}\n  ```\n- next");
    assert_eq!(items.len(), 2);
    let kinds: Vec<&str> = items[0]
        .blocks
        .iter()
        .map(|block| match block {
            Block::Paragraph { .. } => "p",
            Block::Code { .. } => "code",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["p", "p", "code"]);
    assert!(matches!(
        &items[0].blocks[2],
        Block::Code { lang: Some(lang), text } if lang == "rust" && text == "fn x() {}"
    ));
}

#[test]
fn task_items_know_whether_they_are_done() {
    let (_, items) = first_list("- [x] done\n- [ ] open\n- plain\n- [X] upper");
    let tasks: Vec<Option<bool>> = items.iter().map(|item| item.task).collect();
    assert_eq!(tasks, [Some(true), Some(false), None, Some(true)]);
    assert!(matches!(&items[0].blocks[0], Block::Paragraph { runs, .. } if plain(runs) == "done"));
}

#[test]
fn task_marks_survive_in_a_loose_list_and_with_nested_items() {
    let (_, items) = first_list("- [x] one\n\n- [ ] two\n\n  more\n\n- plain");
    let tasks: Vec<Option<bool>> = items.iter().map(|item| item.task).collect();
    assert_eq!(tasks, [Some(true), Some(false), None]);
    assert!(matches!(&items[1].blocks[0], Block::Paragraph { runs, .. } if plain(runs) == "two"));
    assert_eq!(items[1].blocks.len(), 2);
    let (_, nested) = first_list("- a\n  - [ ] b\n  - [x] c");
    let Block::List { items: inner, .. } = &nested[0].blocks[1] else { panic!() };
    assert_eq!(inner.iter().map(|item| item.task).collect::<Vec<_>>(), [Some(false), Some(true)]);
}

#[test]
fn headings_run_from_one_to_six() {
    let tree = parse("# a\n## b\n### c\n#### d\n##### e\n###### f");
    let levels: Vec<u8> = tree
        .iter()
        .map(|block| match block {
            Block::Heading { level, .. } => *level,
            _ => 0,
        })
        .collect();
    assert_eq!(levels, [1, 2, 3, 4, 5, 6]);
}

#[test]
fn rules_and_hard_and_soft_breaks() {
    let tree = parse("a\nb  \nc\n\n---\n\nd");
    assert!(matches!(tree[1], Block::Rule));
    let Block::Paragraph { runs, .. } = &tree[0] else { panic!() };
    assert_eq!(plain(runs), "a b\nc");
}

#[test]
fn quotes_nest_and_hold_blocks() {
    let tree = parse("> one\n> > two\n> > - item");
    let Block::Quote(outer) = &tree[0] else { panic!("{tree:?}") };
    let Block::Quote(inner) = &outer[1] else { panic!("{outer:?}") };
    assert!(matches!(inner[1], Block::List { .. }));
}

#[test]
fn inline_styles_links_escapes_and_code() {
    let tree = parse("**b** *i* ***bi*** ~~s~~ `c` [l](https://x.dev) <https://y.dev> \\*star\\*");
    let Block::Paragraph { runs, .. } = &tree[0] else { panic!() };
    let find =
        |text: &str| runs.iter().find(|run| run.text == text).unwrap_or_else(|| panic!("{text}"));
    assert!(find("b").style.bold && !find("b").style.italic);
    assert!(find("i").style.italic && !find("i").style.bold);
    assert!(find("bi").style.bold && find("bi").style.italic);
    assert!(find("s").style.strike);
    assert_eq!(find("c").kind, Kind::Code);
    assert_eq!(find("l").link.as_deref(), Some("https://x.dev"));
    assert_eq!(find("https://y.dev").link.as_deref(), Some("https://y.dev"));
    assert!(plain(runs).ends_with("*star*"));
}

#[test]
fn an_image_becomes_a_chip_run_with_its_alt_text() {
    let tree = parse("see ![diagram](https://x.dev/a.png) here");
    let Block::Paragraph { runs, .. } = &tree[0] else { panic!() };
    let chip = runs.iter().find(|run| run.kind == Kind::Image).expect("an image run");
    assert_eq!(chip.text, "diagram");
    assert_eq!(chip.link.as_deref(), Some("https://x.dev/a.png"));
}

#[test]
fn html_stays_inert_text_and_angle_brackets_in_prose_survive() {
    let tree = parse("use Vec<String> and <span>x</span>\n\n<div>block</div>");
    assert_eq!(
        plain(match &tree[0] {
            Block::Paragraph { runs, .. } => runs,
            other => panic!("{other:?}"),
        }),
        "use Vec<String> and <span>x</span>"
    );
    assert!(matches!(&tree[1], Block::Paragraph { runs, .. } if plain(runs) == "<div>block</div>"));
}

#[test]
fn tables_keep_alignment_head_and_rows() {
    let tree = parse("| a | b | c |\n|:--|:-:|--:|\n| 1 | 2 | 3 |\n| 4 | 5 | 6 |");
    let Block::Table { aligns, head, rows } = &tree[0] else { panic!("{tree:?}") };
    assert_eq!(aligns, &[Alignment::Left, Alignment::Center, Alignment::Right]);
    assert_eq!(head.len(), 3);
    assert_eq!(rows.len(), 2);
    assert_eq!(plain(&rows[1][2]), "6");
}

#[test]
fn fenced_and_indented_code_blocks() {
    let tree = parse("```sh extra\ncargo test\n```\n\ntext\n\n    indented\n");
    assert!(
        matches!(&tree[0], Block::Code { lang: Some(lang), text } if lang == "sh" && text == "cargo test")
    );
    assert!(matches!(&tree[2], Block::Code { lang: None, text } if text == "indented"));
}

#[test]
fn footnote_definitions_move_to_the_end_and_references_are_note_runs() {
    let tree = parse("Claim[^1] stands.\n\n[^1]: The proof.\n\nAfter.");
    let Block::Paragraph { runs, .. } = &tree[0] else { panic!() };
    assert!(runs.iter().any(|run| run.kind == Kind::Note && run.text == "1"));
    assert_eq!(tree.len(), 3);
    let Some(Block::Notes(notes)) = tree.last() else { panic!("{tree:?}") };
    assert_eq!(notes[0].label, "1");
    assert!(matches!(&tree[1], Block::Paragraph { runs, .. } if plain(runs) == "After."));
}

#[test]
fn empty_and_blank_input_give_no_blocks() {
    assert_eq!(parse("").len(), 0);
    assert_eq!(parse("\n\n  \n").len(), 0);
}
