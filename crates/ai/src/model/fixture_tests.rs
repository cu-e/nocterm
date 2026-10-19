//! Saved version-one transcripts read and write as neutral records without changing a byte.
use super::{content::Content, tools::ToolRecord};
use serde_json::Value;

const CANONICAL: &[u8] = include_bytes!("../history/fixtures/v1/canonical.json");

fn entries() -> Vec<Value> {
    let file: Value = serde_json::from_slice(CANONICAL).unwrap();
    file["entries"].as_array().unwrap().clone()
}

fn same_bytes<T: serde::Serialize + serde::de::DeserializeOwned>(saved: &Value) {
    let record: T = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(
        serde_json::to_vec(&record).unwrap(),
        serde_json::to_vec(saved).unwrap()
    );
}

#[test]
fn saved_user_blocks_and_content_entries_reencode_identically() {
    let mut seen = 0;
    for entry in entries() {
        match entry["kind"].as_str().unwrap() {
            "user" => {
                for block in entry["value"].as_array().unwrap() {
                    same_bytes::<Content>(block);
                    seen += 1;
                }
            }
            "content" => {
                same_bytes::<Content>(&entry["value"]);
                seen += 1;
            }
            _ => {}
        }
    }
    assert!(seen >= 6);
}

#[test]
fn saved_tool_entries_reencode_identically() {
    let mut seen = 0;
    for entry in entries() {
        if entry["kind"] == "tool" {
            same_bytes::<ToolRecord>(&entry["value"]);
            seen += 1;
        }
    }
    assert!(seen >= 1);
}
