mod bindings;

use bindings::exports::ntwk::theater::actor::Guest as ActorGuest;
use bindings::exports::ntwk::theater::message_server_client::Guest as MessageServerClientGuest;
use bindings::ntwk::theater::filesystem::{read_file, write_file, list_files, create_dir, delete_file};
use bindings::ntwk::theater::message_server_host::request;
use bindings::ntwk::theater::runtime::log;
use bindings::ntwk::theater::types::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize)]
struct InitData {
    child_id: String,
    store_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct State {
    child_id: String,
    store_id: String,
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

#[derive(Debug, Serialize, Deserialize)]
struct ChildMessage {
    child_id: String,
    text: String,
    data: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChainEntry {
    parent: Option<String>,
    id: Option<String>,
    data: MessageData,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum MessageData {
    Chat(Message),
    ChildRollup(Vec<ChildMessage>),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Message {
    role: String,
    content: String,
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
    fn new(child_id: String, store_id: String) -> Self {
        Self {
            child_id,
            store_id,
            base_path: String::from("."), // Default to current directory
            permissions: vec!["read".to_string(), "write".to_string()], // Default permissions
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
        let req = Request {
            _type: "request".to_string(),
            data: Action::Get(id.to_string()),
        };

        let request_bytes = serde_json::to_vec(&req)?;
        let response_bytes = request(&self.store_id, &request_bytes)?;

        let response: Value = serde_json::from_slice(&response_bytes)?;
        if response["status"].as_str() == Some("ok") {
            if let Some(value) = response.get("value") {
                let bytes = value
                    .as_array()
                    .ok_or("Expected byte array")?
                    .iter()
                    .map(|v| v.as_u64().unwrap_or(0) as u8)
                    .collect::<Vec<u8>>();
                let entry: ChainEntry = serde_json::from_slice(&bytes)?;
                return Ok(entry);
            }
        }
        Err("Failed to load message".into())
    }

    fn process_fs_commands(&self, commands: Vec<FsCommand>) -> Vec<String> {
        let mut results = Vec::new();

        for cmd in commands {
            let path = self.resolve_path(&cmd.path);
            
            // Check permissions first
            let operation_allowed = match cmd.operation.as_str() {
                "read-file" | "list-files" => self.permissions.contains(&"read".to_string()),
                "write-file" | "create-dir" => self.permissions.contains(&"write".to_string()),
                "delete-file" => self.permissions.contains(&"write".to_string()),
                _ => false,
            };

            if !operation_allowed {
                results.push(format!("❌ Operation '{}' not permitted", cmd.operation));
                continue;
            }

            let result = match cmd.operation.as_str() {
                "read-file" => {
                    match read_file(&path) {
                        Ok(content) => {
                            if let Ok(content_str) = String::from_utf8(content) {
                                format!("📄 File content of '{}': \n```\n{}\n```", cmd.path, content_str)
                            } else {
                                format!("❌ Failed to decode file content of '{}'", cmd.path)
                            }
                        }
                        Err(e) => format!("❌ Failed to read file '{}': {}", cmd.path, e),
                    }
                }
                "write-file" => {
                    if let Some(content) = cmd.content {
                        match write_file(&path, &content) {
                            Ok(_) => format!("✅ Successfully wrote to file '{}'", cmd.path),
                            Err(e) => format!("❌ Failed to write to file '{}': {}", cmd.path, e),
                        }
                    } else {
                        "❌ No content provided for write operation".to_string()
                    }
                }
                "list-files" => {
                    match list_files(&path) {
                        Ok(files) => {
                            let formatted_files = files.iter()
                                .map(|f| format!("  - {}", f))
                                .collect::<Vec<_>>()
                                .join("\n");
                            format!("📁 Contents of '{}':\n{}", cmd.path, formatted_files)
                        }
                        Err(e) => format!("❌ Failed to list files in '{}': {}", cmd.path, e),
                    }
                }
                "create-dir" => {
                    match create_dir(&path) {
                        Ok(_) => format!("✅ Created directory '{}'", cmd.path),
                        Err(e) => format!("❌ Failed to create directory '{}': {}", cmd.path, e),
                    }
                }
                "delete-file" => {
                    match delete_file(&path) {
                        Ok(_) => format!("✅ Deleted file '{}'", cmd.path),
                        Err(e) => format!("❌ Failed to delete file '{}': {}", cmd.path, e),
                    }
                }
                _ => format!("❌ Unknown operation: {}", cmd.operation),
            };
            results.push(result);
        }

        results
    }

    fn extract_fs_commands(content: &str) -> Vec<FsCommand> {
        let mut commands = Vec::new();
        
        // Extract commands between <fs-command> tags
        let parts: Vec<&str> = content.split("<fs-command>").collect();
        for part in parts.iter().skip(1) {
            if let Some(cmd_end) = part.find("</fs-command>") {
                let cmd_xml = &part[..cmd_end];

                // Parse operation
                if let (Some(op_start), Some(op_end)) = (
                    cmd_xml.find("<operation>"),
                    cmd_xml.find("</operation>"),
                ) {
                    let operation = &cmd_xml[op_start + 11..op_end];

                    // Parse path
                    if let (Some(path_start), Some(path_end)) = (
                        cmd_xml.find("<path>"),
                        cmd_xml.find("</path>"),
                    ) {
                        let path = &cmd_xml[path_start + 6..path_end];

                        // Parse optional content
                        let content = if let (Some(content_start), Some(content_end)) = (
                            cmd_xml.find("<content>"),
                            cmd_xml.find("</content>"),
                        ) {
                            Some(cmd_xml[content_start + 9..content_end].to_string())
                        } else {
                            None
                        };

                        // Parse optional edit parameters
                        let old_text = if let (Some(old_start), Some(old_end)) = (
                            cmd_xml.find("<old_text>"),
                            cmd_xml.find("</old_text>"),
                        ) {
                            Some(cmd_xml[old_start + 10..old_end].to_string())
                        } else {
                            None
                        };

                        let new_text = if let (Some(new_start), Some(new_end)) = (
                            cmd_xml.find("<new_text>"),
                            cmd_xml.find("</new_text>"),
                        ) {
                            Some(cmd_xml[new_start + 10..new_end].to_string())
                        } else {
                            None
                        };

                        commands.push(FsCommand {
                            operation: operation.to_string(),
                            path: path.to_string(),
                            content,
                            old_text,
                            new_text,
                        });
                    }
                }
            }
        }

        commands
    }
}

struct Component;

impl ActorGuest for Component {
    fn init(data: Option<Json>) -> Json {
        log("Initializing filesystem child actor");
        let data = data.unwrap();
        let init_data: InitData = serde_json::from_slice(&data).unwrap();
        
        let initial_state = State::new(init_data.child_id, init_data.store_id);
        
        log("State initialized");
        serde_json::to_vec(&initial_state).unwrap()
    }
}

impl MessageServerClientGuest for Component {
    fn handle_request(msg: Json, state: Json) -> (Json, Json) {
        let state: State = serde_json::from_slice(&state).unwrap();
        let request: Value = serde_json::from_slice(&msg).unwrap();

        match request["msg_type"].as_str() {
            Some("introduction") => {
                // Respond with a welcome message
                let response = ChildMessage {
                    child_id: state.child_id.clone(),
                    text: "🤖 Filesystem operations are now available! You can use commands like:\n\
                          - read-file: Read file contents\n\
                          - write-file: Write to a file\n\
                          - list-files: List directory contents\n\
                          - create-dir: Create a new directory\n\
                          - delete-file: Delete a file\n\n\
                          Use XML syntax like:\n\
                          ```xml\n\
                          <fs-command>\n\
                            <operation>list-files</operation>\n\
                            <path>.</path>\n\
                          </fs-command>\n\
                          ```".to_string(),
                    data: json!({}),
                };

                (serde_json::to_vec(&response).unwrap(), serde_json::to_vec(&state).unwrap())
            }
            Some("head-update") => {
                if let Some(head) = request["data"]["head"].as_str() {
                    log(&format!("Processing head update: {}", head));

                    // Load the message at head
                    match state.load_message(head) {
                        Ok(entry) => {
                            log("Successfully loaded message");
                            // Process based on message type
                            match entry.data {
                                MessageData::Chat(msg) => {
                                    log(&format!("Processing chat message: {}", msg.content));
                                    // Only process user messages
                                    if msg.role == "user" {
                                        // Extract and process commands
                                        let commands = State::extract_fs_commands(&msg.content);
                                        if !commands.is_empty() {
                                            log(&format!("Found {} commands", commands.len()));
                                            let results = state.process_fs_commands(commands);
                                            let response = ChildMessage {
                                                child_id: state.child_id.clone(),
                                                text: results.join("\n\n"),
                                                data: json!({"head": head}),
                                            };
                                            return (serde_json::to_vec(&response).unwrap(), serde_json::to_vec(&state).unwrap());
                                        }
                                    }
                                }
                                MessageData::ChildRollup(_) => {
                                    // Skip processing child rollup messages
                                }
                            }
                        }
                        Err(e) => {
                            log(&format!("Error loading message: {}", e));
                            let response = ChildMessage {
                                child_id: state.child_id.clone(),
                                text: format!("❌ Failed to load message: {}", e),
                                data: json!({"head": head}),
                            };
                            return (serde_json::to_vec(&response).unwrap(), serde_json::to_vec(&state).unwrap());
                        }
                    }
                }

                // Default to empty response if no commands were found
                let response = ChildMessage {
                    child_id: state.child_id.clone(),
                    text: String::new(),
                    data: json!({}),
                };

                (serde_json::to_vec(&response).unwrap(), serde_json::to_vec(&state).unwrap())
            }
            Some(other) => {
                let response = ChildMessage {
                    child_id: state.child_id.clone(),
                    text: format!("❓ Unknown message type: {}", other),
                    data: json!({}),
                };

                (serde_json::to_vec(&response).unwrap(), serde_json::to_vec(&state).unwrap())
            }
            None => {
                let response = ChildMessage {
                    child_id: state.child_id.clone(),
                    text: "❌ No message type provided".to_string(),
                    data: json!({}),
                };

                (serde_json::to_vec(&response).unwrap(), serde_json::to_vec(&state).unwrap())
            }
        }
    }

    fn handle_send(msg: Json, state: Json) -> Json {
        state
    }
}

bindings::export!(Component with_types_in bindings);