//! Where Tidewatch releases come from; checking and installing is `kelp_ui::update::Updater`.

use kelp_core::update::{Release, Source};

pub const REPO: &str = "Hobo-Ware/tidewatch";
pub const CASK: &str = "hobo-ware/tap/tidewatch";
pub const APP_NAME: &str = "Tidewatch";

#[must_use]
pub fn source() -> Source {
    Source::new(REPO, CASK).with_user_agent("tidewatch")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Install,
    NotifyOnly,
}

impl Mode {
    /// Only a macOS copy installed from the Homebrew cask can update itself: Linux has no cask, and
    /// a macOS copy that was dragged out of a zip has no cask to upgrade.
    #[must_use]
    pub fn pick(macos: bool, installed_from_cask: bool) -> Self {
        if macos && installed_from_cask { Self::Install } else { Self::NotifyOnly }
    }
}

/// The release `TIDEWATCH_FAKE_UPDATE` stands for, so the pill and the Settings line can be shown
/// without a published release. Blank means no fake.
#[must_use]
pub fn fake_release(version: Option<&str>) -> Option<Release> {
    let version = version?.trim().trim_start_matches('v');
    (!version.is_empty())
        .then(|| Release { version: version.to_owned(), url: release_page(version) })
}

fn release_page(version: &str) -> String {
    format!("https://github.com/{REPO}/releases/tag/v{version}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use kelp_core::update::version_from_release_url;

    #[test]
    fn the_source_is_the_tidewatch_repo_and_cask() {
        let source = source();
        assert_eq!(source.repo, "Hobo-Ware/tidewatch");
        assert_eq!(source.cask, "hobo-ware/tap/tidewatch");
        assert_eq!(source.user_agent, "tidewatch");
    }

    #[test]
    fn only_a_macos_cask_install_updates_itself() {
        let cases = [
            (true, true, Mode::Install),
            (true, false, Mode::NotifyOnly),
            (false, true, Mode::NotifyOnly),
            (false, false, Mode::NotifyOnly),
        ];
        for (macos, cask, want) in cases {
            assert_eq!(Mode::pick(macos, cask), want, "macos {macos}, cask {cask}");
        }
    }

    #[test]
    fn a_fake_release_names_its_release_page() {
        let release = fake_release(Some("9.9.9")).unwrap();
        assert_eq!(release.version, "9.9.9");
        assert_eq!(release.url, "https://github.com/Hobo-Ware/tidewatch/releases/tag/v9.9.9");
    }

    #[test]
    fn a_fake_release_page_reads_back_as_the_same_version() {
        let release = fake_release(Some(" v0.4.0 ")).unwrap();
        assert_eq!(release.version, "0.4.0");
        assert_eq!(version_from_release_url(&release.url).as_deref(), Some("0.4.0"));
    }

    #[test]
    fn no_value_or_a_blank_one_is_no_fake() {
        assert_eq!(fake_release(None), None);
        assert_eq!(fake_release(Some("")), None);
        assert_eq!(fake_release(Some("  v ")), None);
    }
}
