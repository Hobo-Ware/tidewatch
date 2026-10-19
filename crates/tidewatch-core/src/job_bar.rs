//! Progress lines as a script prints them: `label [####----] 42.1% done/total | note | rate/min |
//! ETA 3m`, and `[n/N] label`. A line that does not look like one is no report.

use crate::job::Report;

const MIN_BAR_CELLS: usize = 8;
const STEPS_NAME: &str = "steps";
const BAR_NAME: &str = "progress";

/// The report in one screen line, if it holds a bar or a step counter. The rate and the ETA the
/// line prints are left out: the job works them out from its own readings.
#[must_use]
pub fn parse(line: &str) -> Option<Report> {
    bar(line).or_else(|| steps(line))
}

fn is_cell(c: char) -> bool {
    matches!(c, '#' | '-' | '=' | '>' | '.' | '\u{2588}' | '\u{2591}' | '\u{2593}')
}

fn bar(line: &str) -> Option<Report> {
    let open = line.find('[')?;
    let inside = &line[open + 1..];
    let close = inside.find(']')?;
    let cells = &inside[..close];
    if cells.chars().count() < MIN_BAR_CELLS || !cells.chars().all(is_cell) {
        return None;
    }
    let (percent, rest) = percent_of(inside[close + 1..].trim_start())?;
    let (done, total) = match counts_of(rest.trim_start()) {
        Some((done, total)) => (done, total),
        None => (percent, 100.0),
    };
    let name = label(&line[..open]);
    let unit = if total == 100.0 && done == percent { "%" } else { "" };
    Some(Report {
        name: if name.is_empty() { BAR_NAME.to_owned() } else { name },
        done: Some(done),
        total: Some(total),
        unit: Some(unit.to_owned()),
        ..Report::default()
    })
}

fn label(before: &str) -> String {
    let start = before.find(char::is_alphanumeric).unwrap_or(before.len());
    before[start..].trim().to_owned()
}

fn percent_of(text: &str) -> Option<(f64, &str)> {
    let end = text.find('%')?;
    let number = text[..end].trim();
    if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    Some((number.parse().ok()?, &text[end + 1..]))
}

fn counts_of(text: &str) -> Option<(f64, f64)> {
    let token = text.split_whitespace().next()?;
    let (done, total) = token.split_once('/')?;
    let number = |part: &str| {
        let digits = part.replace(',', "");
        (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
            .then(|| digits.parse::<f64>().ok())
            .flatten()
    };
    Some((number(done)?, number(total)?))
}

fn steps(line: &str) -> Option<Report> {
    let start = line.find(|c: char| c.is_alphanumeric() || c == '[')?;
    let rest = line[start..].strip_prefix('[')?;
    let (counter, after) = rest.split_once(']')?;
    let (done, total) = counts_of(counter).filter(|(_, total)| *total > 0.0)?;
    let phase = after
        .split(['|', '\u{2014}'])
        .next()
        .unwrap_or_default()
        .split("  ")
        .next()
        .unwrap_or_default()
        .trim();
    Some(Report {
        name: STEPS_NAME.to_owned(),
        done: Some(done),
        total: Some(total),
        phase: (!phase.is_empty()).then(|| phase.to_owned()),
        ..Report::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(line: &str) -> Option<(String, f64, f64)> {
        let report = parse(line)?;
        Some((report.name, report.done?, report.total?))
    }

    fn row(name: &str, done: f64, total: f64) -> (String, f64, f64) {
        (name.to_owned(), done, total)
    }

    #[test]
    fn the_house_bar_gives_label_done_and_total() {
        let cases = [
            (
                "orders   [####------------------------] 21.2% 12431/58650 | 412/min | ETA 1h 52m",
                row("orders", 12431.0, 58650.0),
            ),
            (
                "users    [############------------------] 39.9% 8755/21940 | 53.0/min | ETA 4h 8m",
                row("users", 8755.0, 21940.0),
            ),
            (
                "export   [------------------------------] 0.0% 123/308606 | 85 kept | 479.3/min | ETA 10h 43m",
                row("export", 123.0, 308_606.0),
            ),
            (
                "commit   [##############----------------] 45.9% 1202/2619 | 27.3/min | ETA 51m | completed 663 invalid 495",
                row("commit", 1202.0, 2619.0),
            ),
            (
                "orders   [############################--] 94.8% 445000/469291",
                row("orders", 445_000.0, 469_291.0),
            ),
            (
                "send     [######------------------------] 19.7% 990/5025 | 1182.1/min | ETA 3m | failed 7",
                row("send", 990.0, 5025.0),
            ),
            ("users backfill [#####-----] 50.0% 5/10", row("users backfill", 5.0, 10.0)),
            ("orders [########--] 80% 1,200/1,500", row("orders", 1200.0, 1500.0)),
        ];
        for (line, want) in cases {
            assert_eq!(seen(line), Some(want), "{line}");
        }
    }

    #[test]
    fn the_line_may_be_indented_or_led_by_the_agents_own_glyphs() {
        for line in [
            "  \u{23bf} orders [####----------] 21.2% 12431/58650 | 412/min | ETA 1h52m",
            "\u{2502}   orders [####----------] 21.2% 12431/58650",
            "> orders [####----------] 21.2% 12431/58650",
        ] {
            assert_eq!(seen(line), Some(row("orders", 12431.0, 58650.0)), "{line}");
        }
    }

    #[test]
    fn a_bar_without_a_label_or_counts_still_reads() {
        let report = parse(
            "[#-----------------------] 2.3% 5/215 | filled 0 conflict 0 | 7.2 items/s | ETA 29s",
        )
        .unwrap();
        assert_eq!(
            (report.name.as_str(), report.done, report.total),
            ("progress", Some(5.0), Some(215.0))
        );
        let only_percent = parse("sync [######----] 62%").unwrap();
        assert_eq!(
            (
                only_percent.name.as_str(),
                only_percent.done,
                only_percent.total,
                only_percent.unit.as_deref()
            ),
            ("sync", Some(62.0), Some(100.0), Some("%"))
        );
    }

    #[test]
    fn step_counters_are_one_job_called_steps_with_the_label_as_the_phase() {
        let cases = [
            ("[1/35] Item A  elapsed 3s", ("Item A", 1.0, 35.0)),
            ("[2/6] stats   elapsed 5s, eta 25s", ("stats", 2.0, 6.0)),
            ("  [12/12] done | 4s", ("done", 12.0, 12.0)),
        ];
        for (line, (phase, done, total)) in cases {
            let report = parse(line).unwrap();
            assert_eq!(report.name, "steps", "{line}");
            assert_eq!((report.done, report.total), (Some(done), Some(total)), "{line}");
            assert_eq!(report.phase.as_deref(), Some(phase), "{line}");
        }
    }

    #[test]
    fn other_lines_are_not_progress() {
        for line in [
            "",
            "Compiling serde v1.0.0",
            "ok | 138 passed (869 steps) | 0 failed",
            "epoch 1/20:  16%|\u{2588}\u{258c}        | 84/535 [00:02<00:10, 44.16it/s]",
            "items[1/2] are listed",
            "a [ok] b",
            "orders [####] 50% 5/10",
            "orders [####------] fifty 5/10",
            "orders [abcdefghij] 50% 5/10",
            "[0/0]",
            "[x/5] nope",
            "see [docs/readme] for more",
            "[#########------]",
        ] {
            assert_eq!(parse(line), None, "{line}");
        }
    }

    #[test]
    fn nonsense_totals_come_out_as_they_are_for_the_board_to_clean() {
        assert_eq!(seen("orders [###-------] 100% 373/0"), Some(row("orders", 373.0, 0.0)));
        assert_eq!(
            seen("orders [###-------] 100% 58650/58649"),
            Some(row("orders", 58_650.0, 58_649.0))
        );
    }
}
