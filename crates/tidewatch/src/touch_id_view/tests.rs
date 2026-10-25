use super::*;

#[test]
fn each_state_offers_the_one_action_it_can_take() {
    let cases = [
        (TouchId::Missing, "Set up", Some(Click::SetUp)),
        (TouchId::Installed, "Remove", Some(Click::Remove)),
        (TouchId::Outdated, "Set up again", Some(Click::SetUp)),
        (TouchId::Working, "Working...", None),
    ];
    for (state, button, click) in cases {
        let found = look(state);
        assert_eq!((found.button, found.click), (button, click), "{state:?}");
        assert!(!found.desc.contains(['\u{2014}', '\u{2013}']), "{state:?}");
    }
}

#[test]
fn the_setup_text_says_the_password_is_asked_once() {
    assert!(look(TouchId::Missing).desc.contains("password once"));
}
