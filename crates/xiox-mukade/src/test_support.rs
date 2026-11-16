//! A synthetic Claude Code store shared by the tests of this crate.

use serde_json::{json, Value};
use std::path::Path;

pub(crate) fn write_claude_store(root: &Path, id: &str) {
    let dir = root.join("projects").join("D--proj");
    std::fs::create_dir_all(&dir).unwrap();
    let rec = |v: Value| format!("{v}\n");
    let text = [
        rec(json!({"type": "user", "uuid": "u1", "parentUuid": null, "sessionId": id,
            "timestamp": "2026-11-01T10:00:00.000Z", "cwd": "D:\\proj",
            "message": {"role": "user", "content": "make a folder"}})),
        rec(json!({"type": "assistant", "uuid": "a1", "parentUuid": "u1", "sessionId": id,
            "timestamp": "2026-11-01T10:00:01.000Z", "cwd": "D:\\proj",
            "message": {"role": "assistant", "model": "claude-x", "content": [
                {"type": "tool_use", "id": "toolu_1", "name": "Bash",
                 "input": {"command": "mkdir /d/x"}}]}})),
        rec(json!({"type": "user", "uuid": "u2", "parentUuid": "a1", "sessionId": id,
            "timestamp": "2026-11-01T10:00:02.000Z", "cwd": "D:\\proj",
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_1", "content": "done"}]}})),
    ]
    .concat();
    std::fs::write(dir.join(format!("{id}.jsonl")), text).unwrap();
}


/// A subagent of session `root_id`, in Claude Code's layout:
/// `projects/<project>/<root id>/subagents/agent-<agent id>.jsonl`.
pub(crate) fn write_claude_subagent(root: &Path, root_id: &str, agent_id: &str) {
    let dir = root.join("projects").join("D--proj").join(root_id).join("subagents");
    std::fs::create_dir_all(&dir).unwrap();
    let rec = json!({"type": "user", "uuid": "s1", "parentUuid": null, "sessionId": root_id,
        "isSidechain": true, "agentId": agent_id, "timestamp": "2026-11-01T10:00:05.000Z",
        "cwd": "D:\\proj", "message": {"role": "user", "content": "a subtask"}});
    std::fs::write(dir.join(format!("agent-{agent_id}.jsonl")), format!("{rec}\n")).unwrap();
}
