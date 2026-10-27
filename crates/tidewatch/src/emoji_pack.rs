//! The bundled emoji pictures. `scripts/make-emoji.py` writes a pack: a header, an index of
//! `key, offset, length` and the pictures after it. The key is the emoji's code points in hex
//! joined by `_`, with every variation selector left out (`1f468_200d_1f4bb`, `2764`), so one
//! name serves a character with and without its selector.

use std::collections::HashMap;
use std::sync::OnceLock;

const MAGIC: &[u8] = b"TWEP1";

static STILL: &[u8] = include_bytes!("../assets/emoji/still.pack");

pub(crate) struct Pack {
    entries: HashMap<&'static str, &'static [u8]>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
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
    pub(crate) fn parse(bytes: &'static [u8]) -> Option<Self> {
        let mut reader = Reader { bytes, at: 0 };
        (reader.take(MAGIC.len())? == MAGIC).then_some(())?;
        let count = reader.u32()?;
        let mut index = Vec::with_capacity(count.min(1 << 16));
        for _ in 0..count {
            let key_len = usize::from(*reader.take(1)?.first()?);
            let key = std::str::from_utf8(reader.take(key_len)?).ok()?;
            index.push((key, reader.u32()?, reader.u32()?));
        }
        let base = reader.at;
        let entries = index
            .into_iter()
            .map(|(key, offset, len)| {
                let start = base.checked_add(offset)?;
                Some((key, bytes.get(start..start.checked_add(len)?)?))
            })
            .collect::<Option<_>>()?;
        Some(Self { entries })
    }

    pub(crate) fn get(&self, key: &str) -> Option<&'static [u8]> {
        self.entries.get(key).copied()
    }

    pub(crate) fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

pub(crate) fn still() -> &'static Pack {
    static PACK: OnceLock<Pack> = OnceLock::new();
    PACK.get_or_init(|| Pack::parse(STILL).unwrap_or(Pack { entries: HashMap::new() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut index = Vec::new();
        let mut data = Vec::new();
        for (key, blob) in entries {
            index.push(u8::try_from(key.len()).unwrap());
            index.extend_from_slice(key.as_bytes());
            index.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
            index.extend_from_slice(&u32::try_from(blob.len()).unwrap().to_le_bytes());
            data.extend_from_slice(blob);
        }
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&u32::try_from(entries.len()).unwrap().to_le_bytes());
        out.extend(index);
        out.extend(data);
        out
    }

    fn leak(bytes: Vec<u8>) -> &'static [u8] {
        Box::leak(bytes.into_boxed_slice())
    }

    #[test]
    fn a_pack_hands_back_each_picture_by_its_key() {
        let pack = Pack::parse(leak(pack_of(&[("1f600", b"grin"), ("2764", b"heart")]))).unwrap();
        assert_eq!(pack.get("1f600"), Some(&b"grin"[..]));
        assert_eq!(pack.get("2764"), Some(&b"heart"[..]));
        assert_eq!(pack.get("1f680"), None);
        assert_eq!(pack.len(), 2);
    }

    #[test]
    fn a_cut_or_foreign_pack_is_refused_instead_of_read_past_its_end() {
        let whole = pack_of(&[("1f600", b"grin")]);
        assert!(Pack::parse(leak(whole[..whole.len() - 1].to_vec())).is_none());
        assert!(Pack::parse(leak(b"nope".to_vec())).is_none());
        assert!(Pack::parse(leak(Vec::new())).is_none());
    }

    #[test]
    fn the_bundled_pack_holds_the_emoji_the_chat_draws() {
        let pack = still();
        for key in [
            "1f600",
            "1f680",
            "1f389",
            "2764",
            "1f44d",
            "2705",
            "26a0",
            "1f468_200d_1f4bb",
            "1f1e7_1f1ea",
        ] {
            assert!(pack.contains(key), "{key}");
        }
        assert!(pack.len() > 3900, "{}", pack.len());
    }

    #[test]
    fn every_bundled_picture_is_a_png() {
        let pack = still();
        let png = [0x89, b'P', b'N', b'G'];
        for key in ["1f600", "1f1e7_1f1ea", "2764"] {
            assert_eq!(pack.get(key).unwrap()[..4], png, "{key}");
        }
    }
}
