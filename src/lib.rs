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
pub enum Message {
    User {
        content: String,
    },
    Assistant {
        content: String,
        id: String,
        model: String,
        stop_reason: String,
        stop_sequence: Option<String>,
        message_type: String,
        usage: Usage,
    },
}

impl Message {
    pub fn content(&self) -> &str {
        match self {
            Self::User { content } => content,
            Self::Assistant { content, .. } => content,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ChildMessage {
    child_id: String,
    text: String,
    html: Option<String>,
    parent_id: Option<String>,
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

    fn process_fs_commands(&self, commands: Vec<FsCommand>) -> Vec<(String, String)> {
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
                results.push((cmd.operation.clone(), format!("Operation '{}' not permitted", cmd.operation)));
                continue;
            }

            let result = match cmd.operation.as_str() {
                "read-file" => match read_file(&path) {
                    Ok(content) => {
                        if let Ok(content_str) = String::from_utf8(content) {
                            (cmd.operation.clone(), format!("Contents of '{}': {}", cmd.path, content_str))
                        } else {
                            (cmd.operation.clone(), format!("Failed to decode file content of '{}'", cmd.path))
                        }
                    }
                    Err(e) => (cmd.operation.clone(), format!("Failed to read file '{}': {}", cmd.path, e)),
                },
                "write-file" => {
                    if let Some(content) = cmd.content {
                        match write_file(&path, &content) {
                            Ok(_) => (cmd.operation.clone(), format!("Successfully wrote to file '{}'", cmd.path)),
                            Err(e) => (cmd.operation.clone(), format!("Failed to write to file '{}': {}", cmd.path, e)),
                        }
                    } else {
                        (cmd.operation.clone(), "No content provided for write operation".to_string())
                    }
                }
                "edit-file" => match (cmd.old_text, cmd.new_text) {
                    (Some(old_text), Some(new_text)) => match read_file(&path) {
                        Ok(content) => {
                            if let Ok(mut content_str) = String::from_utf8(content) {
                                if content_str.contains(&old_text) {
                                    content_str = content_str.replace(&old_text, &new_text);
                                    match write_file(&path, &content_str) {
                                        Ok(_) => (cmd.operation.clone(), format!("Successfully edited file '{}'", cmd.path)),
                                        Err(e) => (cmd.operation.clone(), format!(
                                            "Failed to write edited content to '{}': {}",
                                            cmd.path, e
                                        )),
                                    }
                                } else {
                                    (cmd.operation.clone(), format!("Text to replace not found in '{}'", cmd.path))
                                }
                            } else {
                                (cmd.operation.clone(), format!("Failed to decode file content of '{}'", cmd.path))
                            }
                        }
                        Err(e) => (cmd.operation.clone(), format!("Failed to read file '{}': {}", cmd.path, e)),
                    },
                    _ => {
                        (cmd.operation.clone(), "Both old_text and new_text must be provided for edit operation".to_string())
                    }
                },
                "list-files" => match list_files(&path) {
                    Ok(files) => {
                        let formatted_files = files
                            .iter()
                            .map(|f| format!(" {}", f))
                            .collect::<Vec<_>>()
                            .join("\n");
                        (cmd.operation.clone(), format!("Contents of '{}': {}", cmd.path, formatted_files))
                    }
                    Err(e) => (cmd.operation.clone(), format!("Failed to list files in '{}': {}", cmd.path, e)),
                },
                "create-dir" => match create_dir(&path) {
                    Ok(_) => (cmd.operation.clone(), format!("Created directory '{}'", cmd.path)),
                    Err(e) => (cmd.operation.clone(), format!("Failed to create directory '{}': {}", cmd.path, e)),
                },
                "delete-file" => match delete_file(&path) {
                    Ok(_) => (cmd.operation.clone(), format!("Deleted file '{}'", cmd.path)),
                    Err(e) => (cmd.operation.clone(), format!("Failed to delete file '{}': {}", cmd.path, e)),
                },
                _ => (cmd.operation.clone(), format!("Unknown operation: {}", cmd.operation)),
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
    fn init(data: Option<Json>, params: (String,)) -> Result<(Option<Vec<u8>>,), String> {
        log("Initializing filesystem child actor");
        let initial_state = State::new(data);
        log(&format!(
            "State initialized with name: {}",
            initial_state.name
        ));
        Ok((Some(serde_json::to_vec(&initial_state).unwrap()),))
    }
}

impl MessageServerClientGuest for Component {
    fn handle_request(
        state: Option<Vec<u8>>,
        params: (Vec<u8>,),
    ) -> Result<(Option<Vec<u8>>, (Vec<u8>,)), String> {
        log("Processing message request");
        log(&format!("State: {:?}", state));
        let mut current_state: State = serde_json::from_slice(&state.unwrap()).unwrap();
        log(&format!("Current state: {:?}", current_state));
        let msg = params.0;
        log(&format!(
            "Received message: {}",
            String::from_utf8_lossy(&msg)
        ));
        let request: Value = serde_json::from_slice(&msg).unwrap();
        log(&format!("Received request: {}", request));

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

                        // Create text version
                        let text = "Filesystem operations for '{name}' initialized.

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

Current permissions: {permissions}"
                                .replace("{name}", &current_state.name)
                                .replace("{permissions}", &current_state.permissions.join(", "));

                        // Create HTML version with better styling
                        let html = format!(r#"<div style="background: var(--bg-secondary); border: 1px solid var(--border-color); border-radius: var(--radius-md); padding: 1rem;">
                            <h3 style="color: var(--accent-primary); margin-bottom: 0.75rem;">Filesystem Operations</h3>
                            <p>Operations for <strong>{name}</strong> initialized with permissions: <code>{permissions}</code></p>
                            
                            <div style="margin-top: 1rem;">
                                <h4 style="color: var(--text-primary);">Available Commands:</h4>
                                <ul>
                                    <li><code>read-file</code> - Read file contents (requires 'read')</li>
                                    <li><code>write-file</code> - Write to a file (requires 'write')</li>
                                    <li><code>edit-file</code> - Edit file contents (requires 'write')</li>
                                    <li><code>list-files</code> - List directory contents (requires 'read')</li>
                                    <li><code>create-dir</code> - Create a new directory (requires 'write')</li>
                                    <li><code>delete-file</code> - Delete a file (requires 'write')</li>
                                </ul>
                            </div>
                            
                            <div style="margin-top: 1rem;">
                                <h4 style="color: var(--text-primary);">Command Examples:</h4>
                                <div style="background: var(--bg-tertiary); padding: 0.75rem; border-radius: var(--radius-sm); margin-bottom: 0.75rem;">
                                    <pre style="margin: 0;"><code>&lt;fs-command name="{name}"&gt;
  &lt;operation&gt;list-files&lt;/operation&gt;
  &lt;path&gt;.&lt;/path&gt;
&lt;/fs-command&gt;</code></pre>
                                </div>
                                <div style="background: var(--bg-tertiary); padding: 0.75rem; border-radius: var(--radius-sm);">
                                    <pre style="margin: 0;"><code>&lt;fs-command name="{name}"&gt;
  &lt;operation&gt;read-file&lt;/operation&gt;
  &lt;path&gt;src/file.rs&lt;/path&gt;
&lt;/fs-command&gt;</code></pre>
                                </div>
                            </div>
                        </div>
                        "#, name = &current_state.name, permissions = &current_state.permissions.join(", "));

                        // Get the head ID from the introduction message if available
                        let head_id = data.get("head").and_then(|h| h.as_str()).map(String::from);
                        
                        let response = ChildMessage {
                            child_id: child_id.to_string(),
                            text,
                            html: Some(html),
                            parent_id: head_id,
                            data: json!({}),
                        };

                        return Ok((
                            Some(serde_json::to_vec(&current_state).unwrap()),
                            (serde_json::to_vec(&response).unwrap(),),
                        ));
                    }
                }
                log("Failed to get child_id or store_id from introduction");
                let response = ChildMessage {
                    child_id: current_state.child_id.clone().unwrap_or_default(),
                    text: "Failed to get child_id or store_id from introduction".to_string(),
                    html: Some("<div style=\"color: var(--text-primary); padding: 0.5rem;\"><p>Failed to get child_id or store_id from introduction</p></div>".to_string()),
                    parent_id: None,
                    data: json!({}),
                };
                Ok((
                    Some(serde_json::to_vec(&current_state).unwrap()),
                    (serde_json::to_vec(&response).unwrap(),),
                ))
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
                                    log(&format!("Processing chat message: {}", msg.content()));
                                    let commands = State::extract_fs_commands(
                                        &msg.content(),
                                        &current_state.name,
                                    );
                                    if !commands.is_empty() {
                                        log(&format!(
                                            "Found {} commands for {}",
                                            commands.len(),
                                            current_state.name
                                        ));
                                        let results = current_state.process_fs_commands(commands);
                                        
                                        // Format text results
                                        let results_text = results.iter()
                                            .map(|(op, result)| result.clone())
                                            .collect::<Vec<_>>()
                                            .join("\n\n");
                                        
                                        // Create HTML version with nice formatting based on operation type
                                        let mut html_parts = Vec::new();
                                        
                                        for (op_type, result) in &results {
                                            let (icon, color) = match op_type.as_str() {
                                                "read-file" => ("📄", "#3B82F6"), // Blue for read
                                                "write-file" => ("✏️", "#10B981"), // Green for write
                                                "edit-file" => ("🔄", "#8B5CF6"),   // Purple for edit
                                                "list-files" => ("📁", "#F59E0B"), // Yellow for list
                                                "create-dir" => ("📂", "#10B981"), // Green for create
                                                "delete-file" => ("🗑️", "#EF4444"), // Red for delete
                                                _ => ("❓", "#6B7280"),            // Gray for unknown
                                            };
                                            
                                            html_parts.push(format!(r#"<div style="margin-bottom: 1rem;">
                                                <div style="display: flex; align-items: center; margin-bottom: 0.5rem;">
                                                    <span style="margin-right: 0.5rem;">{icon}</span>
                                                    <span style="color: {color}; font-weight: bold;">{op_type}</span>
                                                </div>
                                                <div style="background: var(--bg-tertiary); padding: 0.75rem; border-radius: var(--radius-sm);">
                                                    <pre style="margin: 0; white-space: pre-wrap;"><code>{result}</code></pre>
                                                </div>
                                            </div>"#, icon = icon, color = color, op_type = op_type, result = result));
                                        }
                                        
                                        let html = format!(r#"<div style="background: var(--bg-secondary); border: 1px solid var(--border-color); border-radius: var(--radius-md); padding: 1rem;">
                                            <h3 style="color: var(--accent-primary); margin-bottom: 0.75rem;">Filesystem Operation Results</h3>
                                            {results_html}
                                        </div>
                                        "#, results_html = html_parts.join(""));
                                        
                                        let response = ChildMessage {
                                            child_id: child_id.clone(),
                                            text: results_text,
                                            html: Some(html),
                                            parent_id: Some(head.to_string()),
                                            data: json!({"head": head}),
                                        };
                                        return Ok((
                                            Some(serde_json::to_vec(&current_state).unwrap()),
                                            (serde_json::to_vec(&response).unwrap(),),
                                        ));
                                    }
                                }
                                MessageData::ChildRollup(_) => {
                                    // Skip processing child rollup messages
                                }
                            }
                        }
                        Err(e) => {
                            log(&format!("Error loading message: {}", e));
                            let error_text = format!("Failed to load message: {}", e);
                            let html = format!(r#"<div style="background: var(--bg-secondary); border: 1px solid var(--border-color); border-radius: var(--radius-md); padding: 1rem;">
                                <h3 style="color: #EF4444; margin-bottom: 0.75rem;">Error</h3>
                                <div style="background: var(--bg-tertiary); padding: 0.75rem; border-radius: var(--radius-sm);">
                                    <p style="margin: 0;">{}</p>
                                </div>
                            </div>
                            "#, error_text);
                            
                            let response = ChildMessage {
                                child_id: child_id.clone(),
                                text: error_text,
                                html: Some(html),
                                parent_id: Some(head.to_string()),
                                data: json!({"head": head}),
                            };
                            return Ok((
                                Some(serde_json::to_vec(&current_state).unwrap()),
                                (serde_json::to_vec(&response).unwrap(),),
                            ));
                        }
                    }
                }

                let response = ChildMessage {
                    child_id: current_state.child_id.clone().unwrap_or_default(),
                    text: String::new(),
                    html: None,
                    parent_id: request["data"]["head"].as_str().map(String::from),
                    data: json!({}),
                };

                Ok((
                    Some(serde_json::to_vec(&current_state).unwrap()),
                    (serde_json::to_vec(&response).unwrap(),),
                ))
            }
            Some(other) => {
                log(&format!("Unknown message type: {}", other));
                let msg = format!("Unknown message type: {}", other);
                let response = ChildMessage {
                    child_id: current_state.child_id.clone().unwrap_or_default(),
                    text: msg.clone(),
                    html: Some(format!("<div style=\"color: var(--text-primary); padding: 0.5rem;\"><p>{}</p></div>", msg)),
                    parent_id: request["data"]["head"].as_str().map(String::from),
                    data: json!({}),
                };
                Ok((
                    Some(serde_json::to_vec(&current_state).unwrap()),
                    (serde_json::to_vec(&response).unwrap(),),
                ))
            }
            None => {
                log("No message type provided");
                let response = ChildMessage {
                    child_id: current_state.child_id.clone().unwrap_or_default(),
                    text: "No message type provided".to_string(),
                    html: Some("<div style=\"color: var(--text-primary); padding: 0.5rem;\"><p>No message type provided</p></div>".to_string()),
                    parent_id: request["data"]["head"].as_str().map(String::from),
                    data: json!({}),
                };
                Ok((
                    Some(serde_json::to_vec(&current_state).unwrap()),
                    (serde_json::to_vec(&response).unwrap(),),
                ))
            }
        }
    }

    fn handle_send(
        state: Option<Vec<u8>>,
        _params: (Vec<u8>,),
    ) -> Result<(Option<Vec<u8>>,), String> {
        Ok((state,))
    }
}

bindings::export!(Component with_types_in bindings);
