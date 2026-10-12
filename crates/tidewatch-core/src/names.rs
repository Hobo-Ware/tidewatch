//! Readable, collision-free names for new worktrees (`brave-otter`), used as the tab title,
//! the folder name and the `agent/<name>` branch. Pure: the caller supplies the seed and the
//! "is this taken" check.

const ADJECTIVES: [&str; 20] = [
    "brave", "calm", "eager", "fancy", "gentle", "happy", "jolly", "keen", "lively", "merry",
    "nimble", "plucky", "quick", "rapid", "sunny", "swift", "tidy", "upbeat", "witty", "zesty",
];

const NOUNS: [&str; 20] = [
    "badger", "beacon", "comet", "dolphin", "falcon", "harbor", "heron", "lantern", "marlin",
    "meadow", "otter", "pebble", "puffin", "reef", "river", "sparrow", "tide", "walrus", "willow",
    "wombat",
];

/// First `adjective-noun` pair from `seed` onward that `is_taken` does not claim. When every pair
/// is taken it adds a numeric suffix (`brave-otter-2`) until one is free.
pub fn pick(seed: u64, is_taken: impl Fn(&str) -> bool) -> String {
    let total = ADJECTIVES.len() * NOUNS.len();
    let start = (seed % total as u64) as usize;
    let pair = |i: usize| {
        let i = (start + i) % total;
        format!("{}-{}", ADJECTIVES[i / NOUNS.len()], NOUNS[i % NOUNS.len()])
    };
    if let Some(name) = (0..total).map(pair).find(|n| !is_taken(n)) {
        return name;
    }
    let base = pair(0);
    let mut n = 2_u32;
    loop {
        let name = format!("{base}-{n}");
        if !is_taken(&name) {
            return name;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn same_seed_gives_same_name() {
        assert_eq!(pick(7, |_| false), pick(7, |_| false));
    }

    #[test]
    fn names_are_lowercase_pairs_safe_for_branches_and_folders() {
        for seed in 0..400 {
            let name = pick(seed, |_| false);
            let (adjective, noun) = name.split_once('-').expect("adjective-noun");
            for part in [adjective, noun] {
                assert!(!part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase()), "{name}");
            }
        }
    }

    #[test]
    fn skips_taken_names() {
        let first = pick(3, |_| false);
        let second = pick(3, |n| n == first);
        assert_ne!(first, second);
    }

    #[test]
    fn every_pair_is_reachable_and_distinct() {
        let all: HashSet<String> = (0..400).map(|seed| pick(seed, |_| false)).collect();
        assert_eq!(all.len(), 400);
    }

    #[test]
    fn falls_back_to_a_numeric_suffix_when_all_pairs_are_taken() {
        let name = pick(0, |n| n.matches('-').count() == 1);
        assert!(name.ends_with("-2"), "{name}");
        let next = pick(0, |n| n.matches('-').count() == 1 || n.ends_with("-2"));
        assert!(next.ends_with("-3"), "{next}");
    }

    #[test]
    fn seed_past_the_table_wraps() {
        assert_eq!(pick(400, |_| false), pick(0, |_| false));
    }
}
