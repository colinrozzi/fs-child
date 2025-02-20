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

    fn process_fs_commands(&self, commands: Vec<FsCommand>) -> Vec<String> {
        let mut results = Vec::new();

        for cmd in commands {
            let path = self.resolve_path(&cmd.path);

            let operation_allowed = match cmd.operation.as_str() {
                "read-file" | "list-files" => self.permissions.contains(&"read".to_string()),
                "write-file" | "create-dir" | "edit-file" => {
                    self.permissions.contains(&"write".to_string())
                }
                "delete-file" => self.permissions.contains(&"write".to_string()),
                _ => false,
            };

            if !operation_allowed {
                results.push(format!("Operation '{}' not permitted", cmd.operation));
                continue;
            }

            let result = match cmd.operation.as_str() {
                "read-file" => match read_file(&path) {
                    Ok(content) => {
                        if let Ok(content_str) = String::from_utf8(content) {
                            format!("Contents of '{}': {}", cmd.path, content_str)
                        } else {
                            format!("Failed to decode file content of '{}'", cmd.path)
                        }
                    }
                    Err(e) => format!("Failed to read file '{}': {}", cmd.path, e),
                },
                "write-file" => {
                    if let Some(content) = cmd.content {
                        match write_file(&path, &content) {
                            Ok(_) => format!("Successfully wrote to file '{}'", cmd.path),
                            Err(e) => format!("Failed to write to file '{}': {}", cmd.path, e),
                        }
                    } else {
                        "No content provided for write operation".to_string()
                    }
                }
                "edit-file" => match (cmd.old_text, cmd.new_text) {
                    (Some(old_text), Some(new_text)) => match read_file(&path) {
                        Ok(content) => {
                            if let Ok(mut content_str) = String::from_utf8(content) {
                                if content_str.contains(&old_text) {
                                    content_str = content_str.replace(&old_text, &new_text);
                                    match write_file(&path, &content_str) {
                                        Ok(_) => format!("Successfully edited file '{}'", cmd.path),
                                        Err(e) => format!(
                                            "Failed to write edited content to '{}': {}",
                                            cmd.path, e
                                        ),
                                    }
                                } else {
                                    format!("Text to replace not found in '{}'", cmd.path)
                                }
                            } else {
                                format!("Failed to decode file content of '{}'", cmd.path)
                            }
                        }
                        Err(e) => format!("Failed to read file '{}': {}", cmd.path, e),
                    },
                    _ => {
                        "Both old_text and new_text must be provided for edit operation".to_string()
                    }
                },
                "list-files" => match list_files(&path) {
                    Ok(files) => {
                        let formatted_files = files
                            .iter()
                            .map(|f| format!(" {}", f))
                            .collect::<Vec<_>>()
                            .join("\n");
                        format!("Contents of '{}': {}", cmd.path, formatted_files)
                    }
                    Err(e) => format!("Failed to list files in '{}': {}", cmd.path, e),
                },
                "create-dir" => match create_dir(&path) {
                    Ok(_) => format!("Created directory '{}'", cmd.path),
                    Err(e) => format!("Failed to create directory '{}': {}", cmd.path, e),
                },
                "delete-file" => match delete_file(&path) {
                    Ok(_) => format!("Deleted file '{}'", cmd.path),
                    Err(e) => format!("Failed to delete file '{}': {}", cmd.path, e),
                },
                _ => format!("Unknown operation: {}", cmd.operation),
            };
            results.push(result);
        }

        results
    }

    fn extract_fs_commands(content: &str, instance_name: &str) -> Vec<FsCommand> {
        let mut commands = Vec::new();

        // Extract commands between named fs-command tags
        let marker = format!("<fs-command name=\"{}\">", instance_name);
        let parts: Vec<&str> = content.split(&marker).collect();

        for part in parts.iter().skip(1) {
            if let Some(cmd_end) = part.find("</fs-command>") {
                let cmd_xml = &part[..cmd_end];

                // Parse operation
                if let (Some(op_start), Some(op_end)) =
                    (cmd_xml.find("<operation>"), cmd_xml.find("</operation>"))
                {
                    let operation = &cmd_xml[op_start + 11..op_end];

                    // Parse path
                    if let (Some(path_start), Some(path_end)) =
                        (cmd_xml.find("<path>"), cmd_xml.find("</path>"))
                    {
                        let path = &cmd_xml[path_start + 6..path_end];

                        // Parse optional content
                        let content = if let (Some(content_start), Some(content_end)) =
                            (cmd_xml.find("<content>"), cmd_xml.find("</content>"))
                        {
                            Some(cmd_xml[content_start + 9..content_end].to_string())
                        } else {
                            None
                        };

                        // Parse optional edit parameters
                        let old_text = if let (Some(old_start), Some(old_end)) =
                            (cmd_xml.find("<old_text>"), cmd_xml.find("</old_text>"))
                        {
                            Some(cmd_xml[old_start + 10..old_end].to_string())
                        } else {
                            None
                        };

                        let new_text = if let (Some(new_start), Some(new_end)) =
                            (cmd_xml.find("<new_text>"), cmd_xml.find("</new_text>"))
                        {
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
        let initial_state = State::new(data);
        log(&format!(
            "State initialized with name: {}",
            initial_state.name
        ));
        serde_json::to_vec(&initial_state).unwrap()
    }
}

impl MessageServerClientGuest for Component {
    fn handle_request(msg: Json, state: Json) -> (Json, Json) {
        let mut current_state: State = serde_json::from_slice(&state).unwrap();
        let request: Value = serde_json::from_slice(&msg).unwrap();

        match request["msg_type"].as_str() {
            Some("introduction") => {
                log("Processing introduction message");
                if let Some(data) = request.get("data") {
                    if let (Some(child_id), Some(store_id)) = (
                        data.get("child_id").and_then(|v| v.as_str()),
                        data.get("store_id").and_then(|v| v.as_str()),
                    ) {
                        current_state.child_id = Some(child_id.to_string());
                        current_state.store_id = Some(store_id.to_string());
                        log(&format!(
                            "Received child_id: {:?} and store_id: {:?}",
                            current_state.child_id, current_state.store_id
                        ));

                        let response = ChildMessage {
                            child_id: child_id.to_string(),
                            text: "Filesystem operations for '{name}' initialized.

Available commands (with required permissions):
- read-file (requires 'read'): Read file contents
- write-file (requires 'write'): Write to a file
- edit-file (requires 'write'): Edit file contents by replacing text
- list-files (requires 'read'): List directory contents
- create-dir (requires 'write'): Create a new directory
- delete-file (requires 'write'): Delete a file

Command formats:

1. List files:
<fs-command name=\"{name}\">
  <operation>list-files</operation>
  <path>.</path>
</fs-command>

2. Read file:
<fs-command name=\"{name}\">
  <operation>read-file</operation>
  <path>src/file.rs</path>
</fs-command>

3. Write file:
<fs-command name=\"{name}\">
  <operation>write-file</operation>
  <path>src/file.rs</path>
  <content>file contents here</content>
</fs-command>

4. Edit file:
<fs-command name=\"{name}\">
  <operation>edit-file</operation>
  <path>src/file.rs</path>
  <old_text>text to find</old_text>
  <new_text>replacement text</new_text>
</fs-command>

5. Create directory:
<fs-command name=\"{name}\">
  <operation>create-dir</operation>
  <path>new_directory</path>
</fs-command>

6. Delete file:
<fs-command name=\"{name}\">
  <operation>delete-file</operation>
  <path>file_to_delete.txt</path>
</fs-command>

Current permissions: {permissions}
Base path: {base_path}"
                                .replace("{name}", &current_state.name)
                                .replace("{permissions}", &current_state.permissions.join(", "))
                                .replace("{base_path}", &current_state.base_path),
                            data: json!({}),
                        };

                        return (
                            serde_json::to_vec(&response).unwrap(),
                            serde_json::to_vec(&current_state).unwrap(),
                        );
                    }
                }
                log("Failed to get child_id or store_id from introduction");
                let response = ChildMessage {
                    child_id: current_state.child_id.clone().unwrap_or_default(),
                    text: "Failed to get child_id or store_id from introduction".to_string(),
                    data: json!({}),
                };
                (
                    serde_json::to_vec(&response).unwrap(),
                    serde_json::to_vec(&current_state).unwrap(),
                )
            }
            Some("head-update") => {
                if let (Some(child_id), Some(head)) = (
                    current_state.child_id.as_ref(),
                    request["data"]["head"].as_str(),
                ) {
                    log(&format!("Processing head update: {}", head));
                    log(&format!("Loading message with ID: {}", head));

                    match current_state.load_message(head) {
                        Ok(entry) => {
                            log("Successfully loaded message");
                            match entry.data {
                                MessageData::Chat(msg) => {
                                    log(&format!("Processing chat message: {}", msg.content));
                                    let commands = State::extract_fs_commands(
                                        &msg.content,
                                        &current_state.name,
                                    );
                                    if !commands.is_empty() {
                                        log(&format!(
                                            "Found {} commands for {}",
                                            commands.len(),
                                            current_state.name
                                        ));
                                        let results = current_state.process_fs_commands(commands);
                                        let response = ChildMessage {
                                            child_id: child_id.clone(),
                                            text: results.join("\n\n"),
                                            data: json!({"head": head}),
                                        };
                                        return (
                                            serde_json::to_vec(&response).unwrap(),
                                            serde_json::to_vec(&current_state).unwrap(),
                                        );
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
                                child_id: child_id.clone(),
                                text: format!("Failed to load message: {}", e),
                                data: json!({"head": head}),
                            };
                            return (
                                serde_json::to_vec(&response).unwrap(),
                                serde_json::to_vec(&current_state).unwrap(),
                            );
                        }
                    }
                }

                let response = ChildMessage {
                    child_id: current_state.child_id.clone().unwrap_or_default(),
                    text: String::new(),
                    data: json!({}),
                };

                (
                    serde_json::to_vec(&response).unwrap(),
                    serde_json::to_vec(&current_state).unwrap(),
                )
            }
            Some(other) => {
                log(&format!("Unknown message type: {}", other));
                let response = ChildMessage {
                    child_id: current_state.child_id.clone().unwrap_or_default(),
                    text: format!("Unknown message type: {}", other),
                    data: json!({}),
                };
                (
                    serde_json::to_vec(&response).unwrap(),
                    serde_json::to_vec(&current_state).unwrap(),
                )
            }
            None => {
                log("No message type provided");
                let response = ChildMessage {
                    child_id: current_state.child_id.clone().unwrap_or_default(),
                    text: "No message type provided".to_string(),
                    data: json!({}),
                };
                (
                    serde_json::to_vec(&response).unwrap(),
                    serde_json::to_vec(&current_state).unwrap(),
                )
            }
        }
    }

    fn handle_send(msg: Json, state: Json) -> Json {
        state
    }
}

bindings::export!(Component with_types_in bindings);
