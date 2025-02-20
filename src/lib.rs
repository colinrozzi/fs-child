mod bindings;

use bindings::exports::ntwk::theater::actor::Guest as ActorGuest;
use bindings::exports::ntwk::theater::message_server_client::Guest as MessageServerClientGuest;
use bindings::ntwk::theater::filesystem::{
    create_dir, delete_file, list_files, read_file, write_file,
};
use bindings::ntwk::theater::message_server_host::request;
use bindings::ntwk::theater::runtime::log;
use bindings::ntwk::theater::types::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize)]
struct State {
    name: String,
    child_id: Option<String>,
    store_id: Option<String>,
    base_path: String,
    permissions: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FsCommand {
    operation: String,
    path: String,
    content: Option<String>,
    old_text: Option<String>,
    new_text: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ChainEntry {
    parent: Option<String>,
    id: Option<String>,
    data: MessageData,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
enum MessageData {
    Chat(Message),
    ChildRollup(Vec<ChildMessage>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Message {
    role: String,
    content: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ChildMessage {
    child_id: String,
    text: String,
    data: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct Request {
    _type: String,
    data: Action,
}

#[derive(Debug, Serialize, Deserialize)]
enum Action {
    Get(String),
}

impl State {
    fn new(init_data: Option<Json>) -> Self {
        if let Some(data) = init_data {
            if let Ok(config) = serde_json::from_slice::<Value>(&data) {
                return Self {
                    name: config["name"].as_str().unwrap_or("default").to_string(),
                    child_id: None,
                    store_id: None,
                    base_path: config["base_path"].as_str().unwrap_or(".").to_string(),
                    permissions: config["permissions"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_else(|| vec!["read".to_string(), "write".to_string()]),
                };
            }
        }
        Self {
            name: "default".to_string(),
            child_id: None,
            store_id: None,
            base_path: String::from("."),
            permissions: vec!["read".to_string(), "write".to_string()],
        }
    }

    fn resolve_path(&self, relative_path: &str) -> String {
        if relative_path.starts_with("/") {
            relative_path.to_string()
        } else {
            format!("{}/{}", self.base_path, relative_path)
        }
    }

    fn load_message(&self, id: &str) -> Result<ChainEntry, Box<dyn std::error::Error>> {
        let store_id = self.store_id.as_ref().ok_or("Store ID not set")?;

        let req = Request {
            _type: "request".to_string(),
            data: Action::Get(id.to_string()),
        };

        let request_bytes = serde_json::to_vec(&req)?;
        let response_bytes = request(store_id, &request_bytes)?;

        log(&format!(
            "Response: {}",
            String::from_utf8_lossy(&response_bytes)
        ));

        let response: Value = serde_json::from_slice(&response_bytes)?;
        if response["status"].as_str() == Some("ok") {
            if let Some(value) = response
                .get("data")
                .and_then(|d| d.get("Get"))
                .and_then(|g| g.get("value"))
            {
                let bytes = value
                    .as_array()
                    .ok_or("Expected byte array")?
                    .iter()
                    .map(|v| v.as_u64().unwrap_or(0) as u8)
                    .collect::<Vec<u8>>();

                log(&format!(
                    "Decoded message bytes: {}",
                    String::from_utf8_lossy(&bytes)
                ));

                let entry: ChainEntry = serde_json::from_slice(&bytes)?;
                return Ok(entry);
            }
        }
        Err("Failed to load message from store".into())
    }