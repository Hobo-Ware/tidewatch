use super::*;

pub(crate) const DATABASE: &str = r##"<svg viewBox="0 0 64 64"><rect width="64" height="64" rx="16" fill="#123b38"/><ellipse cx="32" cy="20" rx="16" ry="6" fill="none" stroke="#2dd4bf" stroke-width="3"/><path d="M16 20v24c0 3.3 7.2 6 16 6s16-2.7 16-6V20" fill="none" stroke="#2dd4bf" stroke-width="3"/><path d="M16 36c5 4 11-4 16 0s11 4 16 0" fill="none" stroke="#99f6e4" stroke-width="3"/></svg>"##;

fn keeps(svg: &str) -> String {
    sanitize(svg).unwrap_or_else(|e| panic!("{svg}: {e}"))
}

#[test]
fn a_plain_icon_comes_out_with_the_namespace_and_the_same_shapes() {
    let out = keeps(DATABASE);
    assert!(out.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 64 64\">"));
    assert!(out.contains("<rect width=\"64\" height=\"64\" rx=\"16\" fill=\"#123b38\"></rect>"));
    assert!(out.contains("<ellipse cx=\"32\" cy=\"20\" rx=\"16\" ry=\"6\" fill=\"none\""));
    assert_eq!(out.matches("<path").count(), 2);
    assert!(out.ends_with("</svg>"));
}

#[test]
fn clean_output_is_a_fixed_point() {
    let once = keeps(DATABASE);
    assert_eq!(keeps(&once), once);
}

#[test]
fn every_allowed_element_and_attribute_survives() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" opacity="0.9"><g transform="translate(2 3) rotate(45)" fill="red" stroke="rgb(1,2,3)" stroke-width="2" stroke-linecap="round" stroke-linejoin="bevel" fill-opacity="0.5" stroke-opacity="1"><rect x="1" y="2" width="3" height="4" rx="1" ry="1"/><circle cx="5" cy="5" r="2"/><ellipse cx="5" cy="5" rx="2" ry="3"/><line x1="0" y1="0" x2="4" y2="4"/><polyline points="0,0 1,1 2,0"/><polygon points="0 0 4 0 2 3"/><path d="M0 0L4 4Z"/></g></svg>"#;
    let out = keeps(svg);
    for needle in [
        "viewBox=\"0 0 32 32\"",
        "opacity=\"0.9\"",
        "<g fill=\"red\" stroke=\"rgb(1,2,3)\"",
        "transform=\"translate(2 3) rotate(45)\"",
        "stroke-linejoin=\"bevel\"",
        "<rect x=\"1\" y=\"2\" width=\"3\" height=\"4\" rx=\"1\" ry=\"1\">",
        "<circle cx=\"5\" cy=\"5\" r=\"2\">",
        "<ellipse cx=\"5\" cy=\"5\" rx=\"2\" ry=\"3\">",
        "<line x1=\"0\" y1=\"0\" x2=\"4\" y2=\"4\">",
        "<polyline points=\"0,0 1,1 2,0\">",
        "<polygon points=\"0 0 4 0 2 3\">",
        "<path d=\"M0 0L4 4Z\">",
    ] {
        assert!(out.contains(needle), "{needle} in {out}");
    }
}

#[test]
fn hostile_markup_is_dropped_and_the_shape_beside_it_stays() {
    let circle = "<circle cx=\"8\" cy=\"8\" r=\"4\"/>";
    let cases = [
        ("script", format!("<svg><script>alert(1)</script>{circle}</svg>")),
        ("script in a group", format!("<svg><g><script>alert(1)</script>{circle}</g></svg>")),
        ("onload", format!("<svg onload=\"alert(1)\">{circle}</svg>")),
        ("onclick on a shape", "<svg><circle cx=\"8\" cy=\"8\" r=\"4\" onclick=\"x()\"/></svg>".to_owned()),
        ("use with href", format!("<svg><use href=\"http://x.test/a.svg#b\"/>{circle}</svg>")),
        ("use with xlink", format!("<svg xmlns:xlink=\"http://www.w3.org/1999/xlink\"><use xlink:href=\"#a\"/>{circle}</svg>")),
        ("image with data uri", format!("<svg><image href=\"data:image/png;base64,AAAA\" width=\"9\" height=\"9\"/>{circle}</svg>")),
        ("foreignObject", format!("<svg><foreignObject><div xmlns=\"http://www.w3.org/1999/xhtml\">hi</div></foreignObject>{circle}</svg>")),
        ("text", format!("<svg><text x=\"1\" y=\"9\">hello</text>{circle}</svg>")),
        ("style element", format!("<svg><style>circle{{fill:url(http://x.test/a)}}</style>{circle}</svg>")),
        ("style attribute", "<svg><circle cx=\"8\" cy=\"8\" r=\"4\" style=\"fill:url(http://x.test/a)\"/></svg>".to_owned()),
        ("defs and gradient", format!("<svg><defs><linearGradient id=\"g\"/></defs>{circle}</svg>")),
        ("a link", format!("<svg><a href=\"javascript:alert(1)\"><text>x</text></a>{circle}</svg>")),
        ("animate", format!("<svg><animate attributeName=\"r\" to=\"9\"/>{circle}</svg>")),
        ("set", format!("<svg><set attributeName=\"href\" to=\"javascript:1\"/>{circle}</svg>")),
        ("comment and instruction", format!("<svg><!-- hi --><?xml-stylesheet href=\"x\"?>{circle}</svg>")),
        ("other namespace", format!("<svg><x:circle xmlns:x=\"http://evil.test/\" cx=\"1\" cy=\"1\" r=\"1\"/>{circle}</svg>")),
        ("class and id", "<svg id=\"a\" class=\"b\"><circle id=\"c\" class=\"d\" cx=\"8\" cy=\"8\" r=\"4\"/></svg>".to_owned()),
    ];
    for (what, svg) in cases {
        let out = keeps(&svg);
        for banned in [
            "script",
            "onload",
            "onclick",
            "href",
            "image",
            "foreignObject",
            "text",
            "style",
            "defs",
            "gradient",
            "animate",
            "<set",
            "<a ",
            "<use",
            "evil",
            "alert",
            "class=",
            "id=",
            "data:",
            "javascript",
        ] {
            assert!(!out.contains(banned), "{what}: {banned} in {out}");
        }
        assert!(
            out.contains("<circle cx=\"8\" cy=\"8\" r=\"4\">") || what.contains("class"),
            "{what}: {out}"
        );
    }
}

#[test]
fn bad_attribute_values_are_dropped_not_cleaned_up() {
    let cases = [
        "fill=\"url(#a)\"",
        "fill=\"url(http://x.test/a.svg#b)\"",
        "fill=\"javascript:alert(1)\"",
        "fill=\"currentColor\"",
        "fill=\"#12\"",
        "fill=\"#ggg\"",
        "fill=\"rgb(1,2,3);background:url(x)\"",
        "stroke=\"var(--x)\"",
        "stroke-width=\"2px\"",
        "stroke-width=\"-1\"",
        "stroke-linecap=\"arrow\"",
        "opacity=\"2\"",
        "opacity=\"NaN\"",
        "fill-opacity=\"calc(1)\"",
        "transform=\"translate(1) skew(3)\"",
        "transform=\"url(#a)\"",
        "transform=\"translate(1\"",
        "transform=\"translate(a b)\"",
    ];
    for attribute in cases {
        let out = keeps(&format!("<svg><circle cx=\"8\" cy=\"8\" r=\"4\" {attribute}/></svg>"));
        let name = attribute.split('=').next().unwrap();
        assert!(!out.contains(&format!(" {name}=")), "{attribute}: {out}");
        assert!(out.contains("<circle cx=\"8\" cy=\"8\" r=\"4\">"), "{attribute}: {out}");
    }
    let out = keeps(
        "<svg><path d=\"M0 0 url(#a)\" fill=\"red\"/><circle cx=\"1\" cy=\"1\" r=\"1\"/></svg>",
    );
    assert!(out.contains("<path fill=\"red\">") && !out.contains("url"), "{out}");
    let out =
        keeps("<svg><polygon points=\"1,2 x\"/><circle cx=\"1\" cy=\"1\" r=\"1e999\"/></svg>");
    assert!(!out.contains("points") && !out.contains("r="), "{out}");
}

#[test]
fn a_view_box_that_is_not_four_positive_numbers_becomes_the_default() {
    for view_box in ["", "0 0 64", "0 0 0 64", "a b c d", "0 0 -4 4", "0 0 64 64 64", "url(#a)"] {
        let out = keeps(&format!(
            "<svg viewBox=\"{view_box}\"><circle cx=\"1\" cy=\"1\" r=\"1\"/></svg>"
        ));
        assert!(out.contains("viewBox=\"0 0 64 64\""), "{view_box:?}: {out}");
    }
    let out = keeps("<svg viewBox=\"0,0,24,24\"><circle cx=\"1\" cy=\"1\" r=\"1\"/></svg>");
    assert!(out.contains("viewBox=\"0,0,24,24\""));
}

#[test]
fn width_height_and_a_missing_namespace_are_handled() {
    let out =
        keeps("<svg width=\"9999\" height=\"9999\"><circle cx=\"1\" cy=\"1\" r=\"1\"/></svg>");
    assert!(!out.contains("9999"));
    assert!(out.contains("xmlns=\"http://www.w3.org/2000/svg\""));
}

#[test]
fn what_cannot_be_an_avatar_is_refused_with_the_reason() {
    let big = format!(
        "<svg><circle cx=\"1\" cy=\"1\" r=\"1\"/><!--{}--></svg>",
        "a".repeat(MAX_SVG_BYTES)
    );
    let cases: Vec<(&str, String, &str)> = vec![
        ("too big", big, "over the 8 KB limit"),
        ("not xml", "hello".into(), "does not parse"),
        ("empty", String::new(), "does not parse"),
        ("unclosed", "<svg><circle cx=\"1\">".into(), "does not parse"),
        ("mismatched", "<svg><g></svg>".into(), "does not parse"),
        ("html", "<html><body/></html>".into(), "root element must be <svg>"),
        ("other root", "<circle cx=\"1\" cy=\"1\" r=\"1\"/>".into(), "root element must be <svg>"),
        (
            "foreign svg root",
            "<svg xmlns=\"http://evil.test/\"><circle cx=\"1\" cy=\"1\" r=\"1\"/></svg>".into(),
            "root element must be <svg>",
        ),
        (
            "nothing allowed",
            "<svg><script>1</script><text>x</text></svg>".into(),
            "no allowed shape",
        ),
        ("only groups", "<svg><g><g/></g></svg>".into(), "no allowed shape"),
        ("empty svg", "<svg/>".into(), "no allowed shape"),
    ];
    for (what, svg, want) in cases {
        let error = sanitize(&svg).expect_err(what).to_string();
        assert!(error.contains(want), "{what}: {error}");
    }
}

#[test]
fn entity_tricks_and_doctypes_are_refused() {
    let laughs = "<?xml version=\"1.0\"?><!DOCTYPE svg [<!ENTITY a \"aaaaaaaaaa\"><!ENTITY b \"&a;&a;&a;&a;&a;&a;&a;&a;\"><!ENTITY c \"&b;&b;&b;&b;&b;&b;&b;&b;\">]><svg><circle cx=\"&c;\" cy=\"1\" r=\"1\"/></svg>";
    let external = "<!DOCTYPE svg SYSTEM \"http://evil.test/x.dtd\"><svg><circle cx=\"1\" cy=\"1\" r=\"1\"/></svg>";
    let xxe = "<!DOCTYPE svg [<!ENTITY x SYSTEM \"file:///etc/passwd\">]><svg><circle cx=\"&x;\" cy=\"1\" r=\"1\"/></svg>";
    for svg in [laughs, external, xxe] {
        assert!(matches!(sanitize(svg), Err(AvatarError::Unreadable(_))), "{svg}");
    }
}

#[test]
fn nesting_and_element_counts_are_capped_without_failing() {
    let deep = format!(
        "<svg>{}<circle cx=\"1\" cy=\"1\" r=\"1\"/>{}<circle cx=\"2\" cy=\"2\" r=\"2\"/></svg>",
        "<g>".repeat(40),
        "</g>".repeat(40)
    );
    let out = keeps(&deep);
    assert!(out.matches("<g>").count() <= MAX_DEPTH);
    let wide = format!("<svg>{}</svg>", "<circle cx=\"1\" cy=\"1\" r=\"1\"/>".repeat(200));
    assert!(wide.len() < MAX_SVG_BYTES);
    assert_eq!(keeps(&wide).matches("<circle").count(), MAX_ELEMENTS);
}

#[test]
fn the_output_stays_in_the_size_of_the_input() {
    let out = keeps(DATABASE);
    assert!(out.len() <= DATABASE.len() * 2);
}

#[test]
fn quotes_and_angle_brackets_in_a_value_cannot_break_out() {
    let out =
        keeps("<svg><circle cx=\"1\" cy=\"1\" r=\"1\" fill=\"red&quot; onload=&quot;x\"/></svg>");
    assert!(!out.contains("onload"), "{out}");
}

#[test]
fn a_call_needs_a_reason_and_the_reason_is_one_line_cut_to_length() {
    let svg = "<svg><circle cx=\"1\" cy=\"1\" r=\"1\"/></svg>";
    assert_eq!(clean(svg, "   "), Err(AvatarError::NoReason));
    assert!(matches!(clean("nope", "ok"), Err(AvatarError::Unreadable(_))));
    let avatar = clean(svg, "  Filling 58k\n old orders:\ta table  ").unwrap();
    assert_eq!(avatar.reason, "Filling 58k old orders: a table");
    let long = clean(svg, &"word ".repeat(100)).unwrap();
    assert_eq!(long.reason.chars().count(), MAX_REASON_CHARS - 1);
}

#[test]
fn an_avatar_round_trips_through_json() {
    let avatar = clean(DATABASE, "a database with a wave").unwrap();
    let text = serde_json::to_string(&avatar).unwrap();
    assert_eq!(serde_json::from_str::<Avatar>(&text).unwrap(), avatar);
}
