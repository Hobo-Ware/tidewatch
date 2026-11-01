//! The bundled Lucide icons (ISC) an agent can pick for its tab with `set_avatar`, and the colors
//! it can paint them in. `scripts/make-icons.py` writes the pack: a header, an index of
//! `name, offset, length` and the icons' shapes after it. An alias shares its icon's bytes.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::avatar_check::hex;

const MAGIC: &[u8] = b"TWLU1";
const MAX_SUGGESTIONS: usize = 8;
const MIN_PART_CHARS: usize = 3;
const VIEW_BOX: &str = "-4 -4 32 32";

static PACK: &[u8] = include_bytes!("../assets/lucide/icons.pack");

/// The colors an icon can take, each a bright ink that passes `avatar_check::check` on the plate.
/// The app's `brand` palette holds the same values for teal, blue, green, amber, orange and red.
pub const COLORS: [(&str, [u8; 3]); 8] = [
    ("teal", [0x2d, 0xd4, 0xbf]),
    ("blue", [0x60, 0xa5, 0xfa]),
    ("green", [0x8f, 0xd1, 0x6a]),
    ("amber", [0xfb, 0xbf, 0x24]),
    ("orange", [0xfb, 0x92, 0x3c]),
    ("red", [0xf8, 0x71, 0x71]),
    ("pink", [0xf4, 0x72, 0xb6]),
    ("purple", [0xc4, 0xb5, 0xfd]),
];

#[must_use]
pub fn color_names() -> Vec<&'static str> {
    COLORS.iter().map(|(name, _)| *name).collect()
}

#[must_use]
pub fn color_hex(name: &str) -> Option<String> {
    let name = name.trim().to_ascii_lowercase();
    COLORS.iter().find(|(known, _)| *known == name).map(|(_, rgb)| hex(*rgb))
}

struct Pack {
    shapes: HashMap<&'static str, &'static str>,
    names: Vec<&'static str>,
}

struct Reader {
    bytes: &'static [u8],
    at: usize,
}

impl Reader {
    fn take(&mut self, len: usize) -> Option<&'static [u8]> {
        let slice = self.bytes.get(self.at..self.at.checked_add(len)?)?;
        self.at += len;
        Some(slice)
    }

    fn u32(&mut self) -> Option<usize> {
        let raw: [u8; 4] = self.take(4)?.try_into().ok()?;
        usize::try_from(u32::from_le_bytes(raw)).ok()
    }
}

impl Pack {
    fn parse(bytes: &'static [u8]) -> Option<Self> {
        let mut reader = Reader { bytes, at: 0 };
        (reader.take(MAGIC.len())? == MAGIC).then_some(())?;
        let count = reader.u32()?;
        let mut index = Vec::with_capacity(count.min(1 << 14));
        for _ in 0..count {
            let name_len = usize::from(*reader.take(1)?.first()?);
            let name = std::str::from_utf8(reader.take(name_len)?).ok()?;
            index.push((name, reader.u32()?, reader.u32()?));
        }
        let base = reader.at;
        let mut shapes = HashMap::with_capacity(index.len());
        let mut names = Vec::with_capacity(index.len());
        for (name, offset, len) in index {
            let start = base.checked_add(offset)?;
            let shape = std::str::from_utf8(bytes.get(start..start.checked_add(len)?)?).ok()?;
            shapes.insert(name, shape);
            names.push(name);
        }
        names.sort_unstable();
        Some(Self { shapes, names })
    }
}

fn pack() -> &'static Pack {
    static BUNDLED: OnceLock<Pack> = OnceLock::new();
    BUNDLED
        .get_or_init(|| Pack::parse(PACK).unwrap_or(Pack { shapes: HashMap::new(), names: vec![] }))
}

/// How many names the pack holds, aliases included.
#[must_use]
pub fn count() -> usize {
    pack().names.len()
}

/// Every name in the pack, in order.
#[must_use]
pub fn names() -> &'static [&'static str] {
    &pack().names
}

/// The name as the pack spells it: lower case, words joined by hyphens.
#[must_use]
pub fn normalize(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace([' ', '_'], "-")
}

/// The icon drawn in `color_hex`, as Lucide draws it: strokes of 2 with round caps on a 24 unit
/// square, inside a view box wide enough that no corner of it leaves the circle the face is cut
/// to. `None` for a name the pack lacks.
#[must_use]
pub fn svg(name: &str, color_hex: &str) -> Option<String> {
    let shapes = pack().shapes.get(normalize(name).as_str())?.replace("currentColor", color_hex);
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{VIEW_BOX}\" fill=\"none\" stroke=\"{color_hex}\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\">{shapes}</svg>"
    ))
}

/// Up to eight names near `name`: those that start with it, then those that contain it, then those
/// that contain one of its hyphen-separated words.
#[must_use]
pub fn suggest(name: &str) -> Vec<&'static str> {
    let name = normalize(name);
    if name.is_empty() {
        return vec![];
    }
    let parts: Vec<&str> =
        name.split('-').filter(|part| part.chars().count() >= MIN_PART_CHARS).collect();
    let tier = |candidate: &str| -> Option<u8> {
        if candidate.starts_with(&name) {
            Some(0)
        } else if candidate.contains(&name) {
            Some(1)
        } else if parts.iter().any(|part| candidate.contains(part)) {
            Some(2)
        } else {
            None
        }
    };
    let mut found: Vec<(u8, &'static str)> =
        names().iter().filter_map(|candidate| Some((tier(candidate)?, *candidate))).collect();
    found.sort_by_key(|(tier, candidate)| (*tier, candidate.len(), *candidate));
    found.into_iter().take(MAX_SUGGESTIONS).map(|(_, candidate)| candidate).collect()
}

#[cfg(test)]
mod tests;
