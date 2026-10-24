use super::*;

#[test]
fn a_no_from_the_person_ends_the_start_and_a_missing_prompt_falls_back() {
    let cases = [
        (-1, Auth::Denied("Touch ID did not recognise you".to_owned())),
        (-2, Auth::Denied("Touch ID was cancelled".to_owned())),
        (-3, Auth::Denied("Touch ID was cancelled".to_owned())),
        (-4, Auth::Denied("Touch ID was cancelled".to_owned())),
        (-9, Auth::Denied("Touch ID was cancelled".to_owned())),
        (-5, Auth::Unavailable("no passcode (-5)".to_owned())),
        (-1004, Auth::Unavailable("no passcode (-1004)".to_owned())),
    ];
    for (code, want) in cases {
        assert_eq!(auth_from_error(code, "no passcode"), want, "{code}");
    }
}
