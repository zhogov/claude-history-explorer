use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "lowercase")]
pub enum LogEntry {
    Summary {
        #[allow(dead_code)]
        summary: String,
    },
    User {
        message: UserMessage,
        timestamp: String,
    },
    Assistant {
        message: AssistantMessage,
        timestamp: String,
    },
    #[serde(rename = "file-history-snapshot")]
    #[allow(dead_code)]
    FileHistorySnapshot {
        #[serde(rename = "messageId")]
        message_id: String,
        snapshot: serde_json::Value,
        #[serde(rename = "isSnapshotUpdate")]
        is_snapshot_update: bool,
    },
    #[allow(dead_code)]
    System {
        subtype: String,
        level: String,
        #[serde(flatten)]
        extra: serde_json::Value,
    },
}

#[derive(Debug, Deserialize)]
pub struct UserMessage {
    #[allow(dead_code)]
    pub role: String,
    pub content: UserContent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum UserContent {
    String(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Deserialize)]
pub struct AssistantMessage {
    #[allow(dead_code)]
    pub role: String,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        #[allow(dead_code)]
        id: String,
        name: String,
        #[allow(dead_code)]
        input: serde_json::Value,
    },
    #[allow(dead_code)]
    ToolResult {
        tool_use_id: String,
        content: serde_json::Value, // Can be string or array of content blocks
    },
    Thinking {
        thinking: String,
        #[allow(dead_code)]
        signature: String,
    },
    #[allow(dead_code)]
    Image {
        source: serde_json::Value,
    },
}

/// Extract text from content blocks, used for both user and assistant messages
pub fn extract_text_from_blocks(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .take(1)
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(100)
        .collect()
}

pub fn extract_text_from_user(message: &UserMessage) -> String {
    match &message.content {
        UserContent::String(text) => text.chars().take(100).collect(),
        UserContent::Blocks(blocks) => extract_text_from_blocks(blocks),
    }
}

pub fn extract_text_from_assistant(message: &AssistantMessage) -> String {
    extract_text_from_blocks(&message.content)
}
