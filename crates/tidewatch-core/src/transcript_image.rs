//! The pictures Claude Code keeps in a user record (Claude Code 2.1.311): a `{"type":"image",
//! "source":{"type":"base64","media_type":...,"data":...}}` content block beside the text that holds
//! `[Image #N]`, and `imagePasteIds` on the record listing the `N` of each image block in order.
//! Pure: the base64 text is kept as it is and decoded only when a picture is drawn or opened.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::user_markup;

/// One pasted picture of a user message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    /// The `N` of its `[Image #N]` marker, when the record tells it.
    pub number: Option<u32>,
    pub media_type: String,
    pub base64: Arc<str>,
    /// Names the picture: the same bytes give the same hash, whatever message they are in.
    pub hash: u64,
}

impl Picture {
    #[must_use]
    pub fn new(number: Option<u32>, media_type: &str, base64: &str) -> Self {
        let digest = Sha256::digest(base64.as_bytes());
        let hash = u64::from_be_bytes(digest[..8].try_into().unwrap_or_default());
        Self { number, media_type: media_type.to_owned(), base64: base64.into(), hash }
    }

    /// The picture's own bytes; `None` when the text is not valid base64.
    #[must_use]
    pub fn bytes(&self) -> Option<Vec<u8>> {
        STANDARD.decode(self.base64.as_bytes()).ok()
    }

    #[must_use]
    pub fn extension(&self) -> &'static str {
        match self.media_type.as_str() {
            "image/jpeg" | "image/jpg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            _ => "png",
        }
    }

    /// The file a picture is written to so a viewer can open it.
    #[must_use]
    pub fn file_name(&self) -> String {
        format!("reader-{:016x}.{}", self.hash, self.extension())
    }
}

/// The pictures of one Claude user record, in the order of its image blocks. A block's number is its
/// entry in `imagePasteIds`, else the matching `[Image #N]` marker of the record's texts, in order.
pub(crate) fn of_record(line: &Value, content: &Value) -> Vec<Picture> {
    let blocks: Vec<&Value> = content.as_array().into_iter().flatten().collect();
    let ids: Vec<u32> = line["imagePasteIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|id| id.as_u64().and_then(|id| u32::try_from(id).ok()))
        .collect();
    let markers: Vec<u32> = blocks
        .iter()
        .filter(|block| block["type"].as_str() == Some("text"))
        .filter_map(|block| block["text"].as_str())
        .flat_map(user_markup::image_numbers)
        .collect();
    let numbers = if ids.is_empty() { markers } else { ids };
    blocks
        .iter()
        .filter(|block| block["type"].as_str() == Some("image"))
        .filter(|block| block["source"]["type"].as_str() == Some("base64"))
        .filter_map(|block| {
            let data = block["source"]["data"].as_str().filter(|data| !data.is_empty())?;
            Some((block["source"]["media_type"].as_str().unwrap_or("image/png"), data))
        })
        .enumerate()
        .map(|(at, (media_type, data))| Picture::new(numbers.get(at).copied(), media_type, data))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGM4YWMDAAMQAUEiFmcFAAAAAElFTkSuQmCC";

    fn block(media_type: &str, data: &str) -> Value {
        json!({"type": "image", "source": {"type": "base64", "media_type": media_type, "data": data}})
    }

    #[test]
    fn the_images_of_a_record_come_in_order_with_the_numbers_of_the_paste_ids() {
        let line = json!({"imagePasteIds": [9, 10]});
        let content = json!([
            {"type": "text", "text": "[Image #9] and [Image #10]"},
            block("image/png", PIXEL),
            block("image/jpeg", "AAAA"),
        ]);
        let got = of_record(&line, &content);
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].number, got[0].media_type.as_str()), (Some(9), "image/png"));
        assert_eq!((got[1].number, got[1].media_type.as_str()), (Some(10), "image/jpeg"));
        assert_ne!(got[0].hash, got[1].hash);
    }

    #[test]
    fn without_paste_ids_the_markers_of_the_text_number_the_images() {
        let content =
            json!([block("image/png", PIXEL), {"type": "text", "text": "look [Image #4]"}]);
        let got = of_record(&json!({}), &content);
        assert_eq!(got.iter().map(|picture| picture.number).collect::<Vec<_>>(), [Some(4)]);
        let bare = of_record(&json!({}), &json!([block("image/png", PIXEL)]));
        assert_eq!(bare[0].number, None);
    }

    #[test]
    fn only_base64_image_blocks_with_data_count() {
        let content = json!([
            {"type": "image", "source": {"type": "url", "url": "https://x.test/a.png"}},
            block("image/png", ""),
            {"type": "text", "text": "hi"},
        ]);
        assert_eq!(of_record(&json!({}), &content), Vec::new());
    }

    #[test]
    fn a_picture_decodes_back_and_names_its_file_by_its_bytes() {
        let picture = Picture::new(Some(1), "image/png", PIXEL);
        assert_eq!(&picture.bytes().unwrap()[..4], b"\x89PNG");
        let same = Picture::new(Some(7), "image/png", PIXEL);
        assert_eq!(picture.file_name(), same.file_name());
        assert_eq!(picture.file_name().rsplit('.').next(), Some("png"));
        assert_eq!(Picture::new(None, "image/jpeg", PIXEL).extension(), "jpg");
        assert_eq!(Picture::new(None, "image/png", "not base64!").bytes(), None);
    }
}
