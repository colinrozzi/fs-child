# FS-Child Actor

A filesystem child actor for the Chat Actor System that provides file and directory operations.

## Features

- Read file contents
- Write to files
- Edit file contents (find & replace)
- List directory contents
- Create directories
- Delete files

## Usage

The FS-Child actor can be invoked using specialized command tags in your messages:

```xml
<fs-command name="default">
  <operation>list-files</operation>
  <path>.</path>
</fs-command>
```

Where `operation` is one of:
- `read-file` - Read a file's contents
- `write-file` - Write content to a file
- `edit-file` - Find and replace text in a file
- `list-files` - List directory contents
- `create-dir` - Create a new directory
- `delete-file` - Delete a file

## Operation Examples

### List Files
```xml
<fs-command name="default">
  <operation>list-files</operation>
  <path>.</path>
</fs-command>
```

### Read File
```xml
<fs-command name="default">
  <operation>read-file</operation>
  <path>src/lib.rs</path>
</fs-command>
```

### Write File
```xml
<fs-command name="default">
  <operation>write-file</operation>
  <path>new-file.txt</path>
  <content>This is the content for the new file.</content>
</fs-command>
```

### Edit File
```xml
<fs-command name="default">
  <operation>edit-file</operation>
  <path>src/lib.rs</path>
  <old_text>text to find</old_text>
  <new_text>replacement text</new_text>
</fs-command>
```

### Create Directory
```xml
<fs-command name="default">
  <operation>create-dir</operation>
  <path>new-directory</path>
</fs-command>
```

### Delete File
```xml
<fs-command name="default">
  <operation>delete-file</operation>
  <path>file-to-delete.txt</path>
</fs-command>
```

## Configuration

The actor is configured through its `init.json` file:

```json
{
    "name": "default",
    "base_path": ".",
    "permissions": ["read", "write"]
}
```

- `name`: The name used in fs-command tags to target this actor
- `base_path`: The base directory for operations (relative paths are based from here)
- `permissions`: What operations are allowed ("read" and/or "write")

## Permissions

To control what the actor can do:

- `read` permission allows: read-file, list-files
- `write` permission allows: write-file, create-dir, edit-file, delete-file
