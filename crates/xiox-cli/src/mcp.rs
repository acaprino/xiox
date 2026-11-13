//! `xiox observe --mcp`: a read-only MCP server on stdio, written by hand (initialize, ping,
//! tools/list, tools/call: one tool, nothing that writes or removes).
//!
//! One JSON-RPC 2.0 message per line in, one per line out; stdout carries nothing else and what is
//! meant for a person goes to stderr. Single-threaded: there is no timer, a check runs only when
//! `tools/call` arrives, and the server ends when stdin does.

use crate::observe::{
    Deps, LineRead, MAX_LINE_BYTES, Tap, check, json_line, read_capped_line, start,
};
use crate::run::{EXIT_ERROR, write_result};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::sync::{Arc, mpsc};
use xiox_observer::{Observer, Params};

/// The newest protocol revision the server speaks; the others it accepts are echoed back.
const LATEST_VERSION: &str = "2025-06-18";
const SUPPORTED_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// The only tool.
pub const TOOL_NAME: &str = "project_leftovers";

const INSTRUCTIONS: &str = "Xiox watches one project on this PC for the leftovers that agent work creates: \
build folders, git worktrees, copies of the repository, folders named like the project, logs and archived \
agent sessions. Call the project_leftovers tool before you declare your work finished, and again after you \
create a git worktree or a copy of the repository. Items with new: true appeared since this server started. \
The tool only reports and never removes anything; to clean up, show the cleanCommand of the report to the \
person instead of running it yourself.";

const TOOL_DESCRIPTION: &str = "Lists what this project has left on this PC since this server started: build \
folders, linked worktrees, copies of the repository, folders named like the project, log files and archived \
agent sessions, each with its path, size, level and reason, and whether it is new since the start. Also \
returns totals, the growth in bytes since the start, attention (thresholds exceeded), stale (true when the \
fresh check failed and the last report is returned) and cleanCommand, a ready xiox clean command for the \
new recommended items. Runs a fresh check on every call. Read-only: it only reports and never removes anything.";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

fn reply(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn failure(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

fn tool_definition() -> Value {
    json!({
        "name": TOOL_NAME,
        "description": TOOL_DESCRIPTION,
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
        "annotations": {"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false},
    })
}

/// A tool result that is an error the agent can read (not a protocol error).
fn tool_error(message: &str) -> Value {
    json!({"content": [{"type": "text", "text": message}], "isError": true})
}

/// The snapshot the process took at its start, on a worker thread, so that the protocol answers
/// (initialize, ping, tools/list) do not wait for it.
enum Baseline {
    /// The worker is still measuring.
    Pending(mpsc::Receiver<Result<Observer, String>>),
    Ready(Observer),
    /// The first analysis failed (or its thread died): each `tools/call` tries again.
    Failed,
}

struct Server {
    baseline: Baseline,
    params: Params,
    deps: Arc<Deps>,
}

impl Server {
    /// One input line: the reply to write, if any (notifications and blank lines get none).
    fn handle(&mut self, line: &[u8], err: &mut dyn Write) -> Option<Value> {
        let line = line.trim_ascii();
        if line.is_empty() {
            return None;
        }
        let Ok(message) = serde_json::from_slice::<Value>(line) else {
            return Some(failure(
                Value::Null,
                PARSE_ERROR,
                "Parse error: the line is not valid JSON",
            ));
        };
        let Value::Object(message) = message else {
            return Some(failure(
                Value::Null,
                INVALID_REQUEST,
                "Invalid Request: expected a JSON object",
            ));
        };
        let id = match message.get("id") {
            None => None,
            Some(id) if id.is_string() || id.is_number() => Some(id.clone()),
            Some(_) => {
                return Some(failure(
                    Value::Null,
                    INVALID_REQUEST,
                    "Invalid Request: id must be a string or a number",
                ));
            }
        };
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            // A response to a request this server never sends: nothing to answer.
            if message.contains_key("result") || message.contains_key("error") {
                return None;
            }
            return Some(failure(
                id.unwrap_or(Value::Null),
                INVALID_REQUEST,
                "Invalid Request: no method",
            ));
        };
        // A notification (no id) is never answered, whatever it says.
        let id = id?;
        let params = message.get("params");
        Some(match method {
            "initialize" => match params {
                None | Some(Value::Object(_)) => reply(id, initialize_result(params)),
                Some(_) => failure(id, INVALID_PARAMS, "Invalid params: expected an object"),
            },
            "ping" => reply(id, json!({})),
            "tools/list" => reply(id, json!({"tools": [tool_definition()]})),
            "tools/call" => self.call(id, params, err),
            other => failure(id, METHOD_NOT_FOUND, format!("Method not found: {other}")),
        })
    }

    fn call(&mut self, id: Value, params: Option<&Value>, err: &mut dyn Write) -> Value {
        let Some(Value::Object(params)) = params else {
            return failure(
                id,
                INVALID_PARAMS,
                "Invalid params: expected an object with the tool name",
            );
        };
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return failure(
                id,
                INVALID_PARAMS,
                "Invalid params: the tool name is missing",
            );
        };
        if name != TOOL_NAME {
            return failure(id, INVALID_PARAMS, format!("Unknown tool: {name}"));
        }
        if !matches!(
            params.get("arguments"),
            None | Some(Value::Null | Value::Object(_))
        ) {
            return failure(
                id,
                INVALID_PARAMS,
                "Invalid params: arguments must be an object",
            );
        }
        reply(id, self.leftovers(err))
    }

    /// Waits (bounded) for the baseline and gives the observer; `Err` is the tool result to answer
    /// with. A failed or dead baseline is tried again here, synchronously.
    fn observer(&mut self, err: &mut dyn Write) -> Result<&mut Observer, Value> {
        if let Baseline::Pending(rx) = &self.baseline {
            match rx.recv_timeout(self.deps.baseline_wait) {
                Ok(Ok(observer)) => self.baseline = Baseline::Ready(observer),
                Ok(Err(message)) => {
                    let _ = writeln!(err, "error: {message} (the next tools/call tries again)");
                    self.baseline = Baseline::Failed;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(tool_error(
                        "The first analysis of the project is still running. Call project_leftovers again in a moment.",
                    ));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = writeln!(
                        err,
                        "error: the first analysis stopped unexpectedly (the next tools/call tries again)"
                    );
                    self.baseline = Baseline::Failed;
                }
            }
        }
        if matches!(self.baseline, Baseline::Failed) {
            match start(&self.deps, self.params) {
                Ok(observer) => self.baseline = Baseline::Ready(observer),
                Err(message) => {
                    let _ = writeln!(err, "error: {message}");
                    return Err(tool_error(&format!(
                        "The project could not be analysed: {message}. No report is available yet."
                    )));
                }
            }
        }
        match &mut self.baseline {
            Baseline::Ready(observer) => Ok(observer),
            _ => Err(tool_error("No report is available yet.")),
        }
    }

    /// A fresh check, then the report. When the check fails the last report goes out with
    /// `stale: true`; when there is no report at all the result is an error the agent can read.
    fn leftovers(&mut self, err: &mut dyn Write) -> Value {
        let deps = Arc::clone(&self.deps);
        let observer = match self.observer(err) {
            Ok(o) => o,
            Err(result) => return result,
        };
        let mut stale = false;
        if let Err(message) = check(observer, &deps) {
            let _ = writeln!(err, "error: check failed: {message}");
            stale = true;
        }
        let report = match serde_json::to_value(observer.report(stale)) {
            Ok(v) => v,
            Err(e) => {
                let _ = writeln!(err, "error: cannot write the report as JSON: {e}");
                return tool_error(&format!("The report cannot be written as JSON: {e}"));
            }
        };
        json!({"content": [{"type": "text", "text": report.to_string()}], "structuredContent": report, "isError": false})
    }
}

fn initialize_result(params: Option<&Value>) -> Value {
    let wanted = params
        .and_then(|p| p.get("protocolVersion"))
        .and_then(Value::as_str);
    let version = wanted
        .filter(|v| SUPPORTED_VERSIONS.contains(v))
        .unwrap_or(LATEST_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "xiox", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    })
}

/// The server. Returns the exit code: 0 on end of input or a closed pipe, 1 when stdin cannot be
/// read or stdout cannot be written. The baseline is taken at once, on a worker thread (so "new"
/// means new since the process started, and `initialize` is not kept waiting by a slow
/// measurement); a `tools/call` waits for it. The worker is not joined: when the input ends the
/// process ends, whatever it is still measuring.
pub fn serve(
    params: Params,
    mut input: impl BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
    deps: Arc<Deps>,
) -> i32 {
    let mut out = Tap::new(out);
    let (tx, rx) = mpsc::channel();
    let worker = {
        let deps = Arc::clone(&deps);
        std::thread::Builder::new()
            .name("xiox-mcp-baseline".into())
            .spawn(move || {
                // A panic becomes an `Err` on the channel (the hook has already told stderr).
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| start(&deps, params)))
                        .unwrap_or_else(|_| {
                            Err("the first analysis stopped on an internal error".to_string())
                        });
                let _ = tx.send(result);
            })
    };
    let baseline = match worker {
        Ok(_) => Baseline::Pending(rx),
        Err(e) => {
            let _ = writeln!(
                err,
                "error: cannot start the analysis thread: {e} (the next tools/call tries again)"
            );
            Baseline::Failed
        }
    };
    let _ = writeln!(
        err,
        "xiox observe --mcp: serving the {TOOL_NAME} tool on stdio. Nothing is removed."
    );
    let mut server = Server {
        baseline,
        params,
        deps,
    };
    loop {
        let answer = match read_capped_line(&mut input, MAX_LINE_BYTES) {
            Ok(LineRead::Eof) => return 0,
            Ok(LineRead::Line(bytes)) => server.handle(&bytes, err),
            Ok(LineRead::TooLong) => Some(failure(
                Value::Null,
                PARSE_ERROR,
                format!("Parse error: the line is longer than {MAX_LINE_BYTES} bytes"),
            )),
            Err(e) => {
                let _ = writeln!(err, "error: cannot read stdin: {e}");
                return EXIT_ERROR;
            }
        };
        if let Some(answer) = answer {
            // `Value` prints compact JSON with every control character escaped: one line.
            if let Err(code) = write_result(&mut out, err, &json_line(&answer)) {
                return code;
            }
            if out.pipe_closed {
                return 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observe::fakes::*;
    use std::io::Cursor;
    use std::time::Duration;
    use xiox_observer::Params;
    use xiox_project::{ProjectScan, Why};

    struct Session {
        code: i32,
        raw: String,
        stderr: String,
    }

    impl Session {
        fn replies(&self) -> Vec<Value> {
            self.raw
                .lines()
                .map(|l| {
                    serde_json::from_str(l)
                        .unwrap_or_else(|e| panic!("stdout line is not JSON: {l:?}: {e}"))
                })
                .collect()
        }
    }

    fn serve_text(input: &str, script: Vec<Result<ProjectScan, String>>) -> Session {
        let d = deps(script, |_| {});
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = serve(
            Params::default(),
            Cursor::new(input.as_bytes().to_vec()),
            &mut out,
            &mut err,
            std::sync::Arc::new(d),
        );
        Session {
            code,
            raw: String::from_utf8(out).unwrap(),
            stderr: String::from_utf8(err).unwrap(),
        }
    }

    fn lines(messages: &[Value]) -> String {
        messages.iter().map(|m| format!("{m}\n")).collect()
    }

    fn request(id: impl Into<Value>, method: &str, params: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": id.into(), "method": method, "params": params})
    }

    fn call(id: i64) -> Value {
        request(
            id,
            "tools/call",
            json!({"name": "project_leftovers", "arguments": {}}),
        )
    }

    fn one_build() -> Vec<Result<ProjectScan, String>> {
        vec![Ok(scan(vec![build()]))]
    }

    fn report_of(reply: &Value) -> &Value {
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        &reply["result"]["structuredContent"]
    }

    #[test]
    fn initialize_negotiates_the_protocol_version_and_has_instructions() {
        for version in ["2025-06-18", "2025-03-26", "2024-11-05"] {
            let init = request(
                1,
                "initialize",
                json!({"protocolVersion": version, "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}),
            );
            let s = serve_text(&lines(&[init]), one_build());
            assert_eq!(s.code, 0, "{}", s.stderr);
            let replies = s.replies();
            assert_eq!(replies.len(), 1, "{}", s.raw);
            let r = &replies[0];
            assert_eq!(r["jsonrpc"], "2.0");
            assert_eq!(r["id"], 1);
            assert!(r.get("error").is_none(), "{r}");
            assert_eq!(r["result"]["protocolVersion"], version);
            assert_eq!(r["result"]["capabilities"], json!({"tools": {}}));
            assert_eq!(r["result"]["serverInfo"]["name"], "xiox");
            assert_eq!(
                r["result"]["serverInfo"]["version"],
                env!("CARGO_PKG_VERSION")
            );
            let text = r["result"]["instructions"].as_str().expect("instructions");
            assert!(text.contains("project_leftovers"), "{text}");
            assert!(
                text.contains("finished") && text.contains("worktree"),
                "{text}"
            );
            assert!(text.contains("never removes"), "{text}");
            assert!(text.is_ascii(), "English only: {text}");
        }
        // A string id is echoed as a string.
        let s = serve_text(
            &lines(&[request(
                "abc",
                "initialize",
                json!({"protocolVersion": "2025-06-18"}),
            )]),
            one_build(),
        );
        assert_eq!(s.replies()[0]["id"], "abc");
    }

    #[test]
    fn initialize_with_an_unknown_version_answers_the_latest() {
        for params in [
            json!({"protocolVersion": "1999-01-01"}),
            json!({"protocolVersion": 7}),
            json!({}),
        ] {
            let s = serve_text(
                &lines(&[request(1, "initialize", params.clone())]),
                one_build(),
            );
            assert_eq!(
                s.replies()[0]["result"]["protocolVersion"],
                "2025-06-18",
                "{params}"
            );
        }
    }

    #[test]
    fn initialized_notification_gets_no_reply() {
        let input = lines(&[
            request(1, "initialize", json!({"protocolVersion": "2025-06-18"})),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "method": "notifications/something-else", "params": {}}),
            request(2, "ping", json!({})),
        ]);
        let s = serve_text(&input, one_build());
        let ids: Vec<Value> = s.replies().iter().map(|r| r["id"].clone()).collect();
        assert_eq!(ids, [json!(1), json!(2)], "{}", s.raw);
    }

    #[test]
    fn ping_works() {
        let s = serve_text(&lines(&[request(9, "ping", json!({}))]), one_build());
        assert_eq!(
            s.replies(),
            [json!({"jsonrpc": "2.0", "id": 9, "result": {}})]
        );
        // A ping without params works too.
        let s = serve_text(
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"ping\"}\n",
            one_build(),
        );
        assert_eq!(s.replies()[0]["result"], json!({}));
    }

    #[test]
    fn tools_list_has_exactly_one_read_only_tool() {
        let s = serve_text(&lines(&[request(1, "tools/list", json!({}))]), one_build());
        let tools = s.replies()[0]["result"]["tools"].clone();
        let tools = tools.as_array().unwrap();
        assert_eq!(tools.len(), 1, "{tools:?}");
        let tool = &tools[0];
        assert_eq!(tool["name"], "project_leftovers");
        assert_eq!(
            tool["inputSchema"],
            json!({"type": "object", "properties": {}, "additionalProperties": false})
        );
        assert_eq!(
            tool["annotations"],
            json!({"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false})
        );
        let description = tool["description"].as_str().unwrap();
        assert!(
            description.is_ascii() && description.contains("never removes"),
            "{description}"
        );
    }

    #[test]
    fn tools_call_returns_the_report_as_text_and_structured_content() {
        let s = serve_text(&lines(&[call(4)]), one_build());
        assert_eq!(s.code, 0, "{}", s.stderr);
        let replies = s.replies();
        assert_eq!(replies.len(), 1, "{}", s.raw);
        let reply = &replies[0];
        assert_eq!(reply["id"], 4);
        let content = reply["result"]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        let text = content[0]["text"].as_str().unwrap();
        assert!(!text.contains('\n'), "compact JSON: {text}");
        let from_text: Value = serde_json::from_str(text).unwrap();
        let structured = report_of(reply);
        assert_eq!(&from_text, structured);
        assert_eq!(structured["schema"], 1);
        assert_eq!(structured["project"]["name"], "mukade");
        assert_eq!(structured["stale"], false);
        assert_eq!(structured["growthBytes"], 0);
        assert_eq!(structured["items"][0]["category"], "builds");
        assert_eq!(structured["items"][0]["new"], false);
        assert_eq!(structured["items"][0]["bytes"], 10 * MB);
        assert_eq!(structured["attention"], json!([]));
        // The server never speaks on its own: no `started` line, no event.
        assert_eq!(replies.len(), 1);
        // `arguments` is optional.
        let bare = request(5, "tools/call", json!({"name": "project_leftovers"}));
        let s = serve_text(&lines(&[bare]), one_build());
        assert_eq!(s.replies()[0]["result"]["isError"], false, "{}", s.raw);
    }

    #[test]
    fn a_new_worktree_between_two_calls_shows_up_as_new_in_the_second() {
        let script = vec![
            Ok(scan(vec![build()])),                        // baseline, at the start
            Ok(scan(vec![build()])),                        // first call: nothing new
            Ok(scan(vec![build(), worktree("mukade-wt")])), // second call: a worktree
        ];
        let s = serve_text(&lines(&[call(1), call(2)]), script);
        let replies = s.replies();
        assert_eq!(replies.len(), 2, "{}", s.raw);
        let first = report_of(&replies[0]);
        assert!(
            first["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|i| i["new"] == false),
            "{first}"
        );
        let second = report_of(&replies[1]);
        let items = second["items"].as_array().unwrap();
        let wt = items
            .iter()
            .find(|i| i["category"] == "worktrees")
            .expect("the new worktree");
        assert_eq!(wt["new"], true);
        assert_eq!(
            wt["bytes"],
            10 * MB,
            "a new folder is weighed when first seen"
        );
        assert!(
            items
                .iter()
                .any(|i| i["category"] == "builds" && i["new"] == false)
        );
        assert_eq!(second["growthBytes"], 10 * MB as i64);
        // The baseline is the start of the process, not the first call: a worktree that is there
        // by the first call is new already.
        let s = serve_text(
            &lines(&[call(1)]),
            vec![
                Ok(scan(vec![build()])),
                Ok(scan(vec![build(), worktree("w")])),
            ],
        );
        let first = report_of(&s.replies()[0]).clone();
        assert!(
            first["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["category"] == "worktrees" && i["new"] == true)
        );
    }

    #[test]
    fn failed_check_returns_the_last_report_stale() {
        let script = vec![
            Ok(scan(vec![build()])),
            Ok(scan(vec![build(), worktree("mukade-wt")])),
            Err("disk went away".into()),
        ];
        let s = serve_text(&lines(&[call(1), call(2)]), script);
        let replies = s.replies();
        assert_eq!(replies.len(), 2, "{}", s.raw);
        assert_eq!(report_of(&replies[0])["stale"], false);
        let stale = report_of(&replies[1]);
        assert_eq!(stale["stale"], true);
        // It is the last good report: the worktree seen by the first call is still there.
        assert!(
            stale["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["category"] == "worktrees"),
            "{stale}"
        );
        assert_eq!(replies[1]["result"]["isError"], false);
        // The person is told on stderr, never on stdout.
        assert!(s.stderr.contains("disk went away"), "{}", s.stderr);
        assert!(!s.raw.contains("disk went away"), "{}", s.raw);
    }

    #[test]
    fn failed_check_with_no_report_is_an_error_result() {
        let s = serve_text(
            &lines(&[call(1)]),
            vec![Err("path not found: D:\\nope".into())],
        );
        assert_eq!(s.code, 0, "the server stays up: {}", s.stderr);
        let reply = &s.replies()[0];
        assert!(
            reply.get("error").is_none(),
            "an error RESULT, not a JSON-RPC error: {reply}"
        );
        assert_eq!(reply["result"]["isError"], true);
        assert!(
            reply["result"].get("structuredContent").is_none(),
            "{reply}"
        );
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("path not found") && text.is_ascii(), "{text}");
        assert!(s.stderr.contains("path not found"), "{}", s.stderr);
        // The first analysis is tried again at the next call.
        let s = serve_text(
            &lines(&[call(1), call(2)]),
            vec![
                Err("first".into()),
                Err("second".into()),
                Ok(scan(vec![build()])),
            ],
        );
        let replies = s.replies();
        assert_eq!(replies[0]["result"]["isError"], true);
        assert_eq!(report_of(&replies[1])["items"][0]["category"], "builds");
    }

    #[test]
    fn unknown_tool_unknown_method_and_garbage_line_give_jsonrpc_errors_and_the_server_keeps_running()
     {
        let mut input = String::from("{ this is not json\n");
        input += &lines(&[
            request(2, "resources/list", json!({})),
            request(3, "tools/call", json!({"name": "rm_rf", "arguments": {}})),
            request(4, "tools/call", json!({})),
            request(
                5,
                "tools/call",
                json!({"name": "project_leftovers", "arguments": 7}),
            ),
            json!({"jsonrpc": "2.0", "method": "no/such/notification"}),
            json!([1, 2]),
            json!(42),
            json!({"jsonrpc": "2.0", "id": 8}),
            json!({"jsonrpc": "2.0", "id": 9, "result": {}}),
            request(10, "ping", json!({})),
        ]);
        let s = serve_text(&input, one_build());
        assert_eq!(s.code, 0, "{}", s.stderr);
        let replies = s.replies();
        let error = |i: usize| {
            (
                replies[i]["id"].clone(),
                replies[i]["error"]["code"].clone(),
                &replies[i]["error"]["message"],
            )
        };
        // Garbage: parse error, id null.
        assert_eq!(
            (error(0).0, error(0).1),
            (Value::Null, json!(-32700)),
            "{}",
            s.raw
        );
        assert!(error(0).2.as_str().is_some_and(|m| !m.is_empty()));
        assert_eq!((error(1).0, error(1).1), (json!(2), json!(-32601)));
        assert_eq!((error(2).0, error(2).1), (json!(3), json!(-32602)));
        assert!(error(2).2.as_str().unwrap().contains("rm_rf"));
        assert_eq!((error(3).0, error(3).1), (json!(4), json!(-32602)));
        assert_eq!((error(4).0, error(4).1), (json!(5), json!(-32602)));
        // Not objects, or no method: invalid request. A notification and a client response: no reply.
        assert_eq!((error(5).0, error(5).1), (Value::Null, json!(-32600)));
        assert_eq!((error(6).0, error(6).1), (Value::Null, json!(-32600)));
        assert_eq!((error(7).0, error(7).1), (json!(8), json!(-32600)));
        // And it is still serving.
        assert_eq!(replies.len(), 9, "{}", s.raw);
        assert_eq!(
            replies[8],
            json!({"jsonrpc": "2.0", "id": 10, "result": {}})
        );
        for r in &replies[..8] {
            assert_eq!(r["jsonrpc"], "2.0");
            assert!(r.get("result").is_none(), "{r}");
        }
    }

    #[test]
    fn an_overlong_line_is_a_parse_error_and_the_server_keeps_running() {
        let long = "x".repeat(MAX_LINE_BYTES + 1);
        let input = format!("{long}\n{}", lines(&[request(1, "ping", json!({}))]));
        let s = serve_text(&input, one_build());
        let replies = s.replies();
        assert_eq!(replies.len(), 2, "{}", s.raw);
        assert_eq!(replies[0]["error"]["code"], -32700);
        assert_eq!(replies[0]["id"], Value::Null);
        assert!(replies[0]["error"]["message"].as_str().unwrap().len() < 300);
        assert_eq!(replies[1]["result"], json!({}));
    }

    #[test]
    fn every_output_line_is_a_single_json_object() {
        const BRANCH: &str = "a\nb\"c\u{2028}d\u{2029}e";
        // A branch name with a line break and quotes must come out escaped on one line.
        let mut wt = worktree("mukade-wt");
        wt.why = Why::LinkedWorktree {
            branch: Some(BRANCH.into()),
            head: None,
        };
        let script = vec![
            Ok(scan(vec![build()])),
            Ok(scan(vec![build(), wt])),
            Err("boom\nsecond line".into()),
        ];
        let input = lines(&[
            request(1, "initialize", json!({"protocolVersion": "2025-06-18"})),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            request(2, "tools/list", json!({})),
            call(3),
            call(4),
            request(5, "bogus", json!({})),
            request(6, "ping", json!({})),
        ]) + "garbage\n\r\n   \n";
        let s = serve_text(&input, script);
        assert_eq!(s.code, 0, "{}", s.stderr);
        assert!(s.raw.ends_with('\n'));
        let raw_lines: Vec<&str> = s.raw.lines().collect();
        // 6 requests with an id + 1 garbage line; the notification and the blank lines are silent.
        assert_eq!(raw_lines.len(), 7, "{}", s.raw);
        for line in raw_lines {
            let v: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("{line:?}: {e}"));
            assert!(v.is_object() && v["jsonrpc"] == "2.0", "{line}");
            assert!(
                v.get("result").is_some() ^ v.get("error").is_some(),
                "{line}"
            );
        }
        // U+2028 / U+2029 are escaped on the wire (line splitters break on them) and come back intact.
        assert!(
            !s.raw.contains('\u{2028}') && !s.raw.contains('\u{2029}'),
            "{}",
            s.raw
        );
        assert!(s.raw.contains("\\u2028") && s.raw.contains("\\u2029"));
        fn has_string(v: &Value, wanted: &str) -> bool {
            match v {
                Value::String(s) => s == wanted,
                Value::Array(a) => a.iter().any(|x| has_string(x, wanted)),
                Value::Object(o) => o.values().any(|x| has_string(x, wanted)),
                _ => false,
            }
        }
        assert!(
            has_string(&s.replies()[3], BRANCH),
            "the structured report carries the same string"
        );
        // The multi-line error text reached stderr in one piece of text, never stdout.
        assert!(s.stderr.contains("boom"), "{}", s.stderr);
        assert!(!s.raw.contains("boom"));
    }

    #[test]
    fn eof_ends_the_server() {
        let s = serve_text("", one_build());
        assert_eq!(s.code, 0, "{}", s.stderr);
        assert!(s.raw.is_empty(), "{}", s.raw);
        // The last request may have no line end.
        let s = serve_text(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}",
            one_build(),
        );
        assert_eq!(s.code, 0);
        assert_eq!(s.replies().len(), 1);
        // Even when nothing could be analysed at the start, EOF is a clean end.
        let s = serve_text("", vec![Err("nope".into())]);
        assert_eq!(s.code, 0);
        assert!(s.raw.is_empty());
    }

    #[test]
    fn a_closed_pipe_ends_the_server_with_0_and_another_write_error_with_1() {
        struct Failing(std::io::ErrorKind);
        impl Write for Failing {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from(self.0))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::from(self.0))
            }
        }
        let input = || {
            let ping = format!("{}\n", request(1, "ping", json!({})));
            Cursor::new(ping.repeat(1000))
        };
        let d = std::sync::Arc::new(with_interval(
            deps(one_build(), |_| {}),
            Duration::from_secs(1),
        ));
        let mut err = Vec::new();
        let code = serve(
            Params::default(),
            input(),
            &mut Failing(std::io::ErrorKind::BrokenPipe),
            &mut err,
            d.clone(),
        );
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
        let mut err = Vec::new();
        let code = serve(
            Params::default(),
            input(),
            &mut Failing(std::io::ErrorKind::PermissionDenied),
            &mut err,
            d,
        );
        assert_eq!(code, 1);
        assert!(String::from_utf8_lossy(&err).contains("cannot write to the output"));
    }

    /// A stdin that hands out its lines one by one and runs `before(n)` just before line `n` is read.
    struct Scripted {
        lines: Vec<Vec<u8>>,
        next: usize,
        current: Vec<u8>,
        pos: usize,
        before: Box<dyn FnMut(usize)>,
    }

    impl std::io::Read for Scripted {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = {
                let chunk = self.fill_buf()?;
                let n = chunk.len().min(buf.len());
                buf[..n].copy_from_slice(&chunk[..n]);
                n
            };
            self.consume(n);
            Ok(n)
        }
    }

    impl BufRead for Scripted {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            if self.pos >= self.current.len() {
                self.current.clear();
                self.pos = 0;
                if self.next < self.lines.len() {
                    (self.before)(self.next);
                    self.current = self.lines[self.next].clone();
                    self.next += 1;
                }
            }
            Ok(&self.current[self.pos..])
        }

        fn consume(&mut self, amt: usize) {
            self.pos += amt;
        }
    }

    /// An output the test can look into while the server runs.
    #[derive(Clone, Default)]
    struct Shared(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn protocol_answers_do_not_wait_for_the_baseline_and_a_call_waits_for_it() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex, mpsc};
        // The initial analysis is blocked until the gate opens; the gate opens only after the
        // `tools/call` line has been handed to the server (by another thread, after a pause).
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let gate_rx = Mutex::new(gate_rx);
        let released = Arc::new(AtomicBool::new(false));
        let d = deps(
            vec![
                Ok(scan(vec![build()])), // the baseline, behind the gate
                Ok(scan(vec![build(), worktree("mukade-wt")])), // the check made by the call
            ],
            {
                let released = Arc::clone(&released);
                move |n| {
                    if n == 0 {
                        // A safety net: a failing test must end, not hang.
                        let _ = gate_rx
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(30));
                        released.store(true, Ordering::SeqCst);
                    }
                }
            },
        );
        let out = Shared::default();
        let seen_before_call: Arc<Mutex<Option<(usize, bool)>>> = Arc::default();
        let input = Scripted {
            lines: [
                request(1, "initialize", json!({"protocolVersion": "2025-06-18"})),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                request(2, "ping", json!({})),
                request(3, "tools/list", json!({})),
                call(4),
            ]
            .iter()
            .map(|m| format!("{m}\n").into_bytes())
            .collect(),
            next: 0,
            current: Vec::new(),
            pos: 0,
            before: Box::new({
                let (out, released, seen) = (
                    out.clone(),
                    Arc::clone(&released),
                    Arc::clone(&seen_before_call),
                );
                move |n| {
                    if n == 4 {
                        let answered = out
                            .0
                            .lock()
                            .unwrap()
                            .iter()
                            .filter(|&&b| b == b'\n')
                            .count();
                        *seen.lock().unwrap() = Some((answered, released.load(Ordering::SeqCst)));
                        let gate_tx = gate_tx.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(100));
                            let _ = gate_tx.send(());
                        });
                    }
                }
            }),
        };
        let mut err = Vec::new();
        let code = serve(
            Params::default(),
            input,
            &mut out.clone(),
            &mut err,
            Arc::new(d),
        );
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
        // When the call line was read: initialize, ping and tools/list were answered, the baseline analysis still blocked.
        assert_eq!(*seen_before_call.lock().unwrap(), Some((3, false)));
        let raw = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        let replies: Vec<Value> = raw
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let ids: Vec<Value> = replies.iter().map(|r| r["id"].clone()).collect();
        assert_eq!(ids, [json!(1), json!(2), json!(3), json!(4)], "{raw}");
        // The call was answered after the gate, and the baseline is the state at the start: the
        // build folder was there (new: false), the worktree appeared since (new: true).
        let report = report_of(&replies[3]);
        let items = report["items"].as_array().unwrap();
        assert!(
            items
                .iter()
                .any(|i| i["category"] == "builds" && i["new"] == false),
            "{report}"
        );
        assert!(
            items
                .iter()
                .any(|i| i["category"] == "worktrees" && i["new"] == true),
            "{report}"
        );
        assert_eq!(report["stale"], false);
    }

    fn scripted(messages: &[Value], before: impl FnMut(usize) + 'static) -> Scripted {
        Scripted {
            lines: messages
                .iter()
                .map(|m| format!("{m}\n").into_bytes())
                .collect(),
            next: 0,
            current: Vec::new(),
            pos: 0,
            before: Box::new(before),
        }
    }

    /// Fake dependencies whose baseline analysis (the first call) waits for `open` to be sent
    /// (or for 30 s, so that a failing test ends).
    fn gated_deps(
        script: Vec<Result<ProjectScan, String>>,
        wait: Duration,
    ) -> (Deps, std::sync::mpsc::Sender<()>) {
        let (open, gate) = std::sync::mpsc::channel::<()>();
        let gate = std::sync::Mutex::new(gate);
        let mut d = deps(script, move |n| {
            if n == 0 {
                let _ = gate.lock().unwrap().recv_timeout(Duration::from_secs(30));
            }
        });
        d.baseline_wait = wait;
        (d, open)
    }

    fn parse(raw: &[u8]) -> Vec<Value> {
        String::from_utf8_lossy(raw)
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn a_call_that_waits_too_long_for_the_baseline_gets_an_actionable_error_and_a_later_call_succeeds()
     {
        let (d, open) = gated_deps(
            vec![
                Ok(scan(vec![build()])),
                Ok(scan(vec![build(), worktree("mukade-wt")])),
            ],
            Duration::from_millis(100),
        );
        let out = Shared::default();
        let input = scripted(&[call(1), call(2)], move |n| {
            if n == 1 {
                // The baseline finishes between the two calls.
                open.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(400));
            }
        });
        let mut err = Vec::new();
        let code = serve(
            Params::default(),
            input,
            &mut out.clone(),
            &mut err,
            Arc::new(d),
        );
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
        let replies = parse(&out.0.lock().unwrap());
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0]["result"]["isError"], true, "{}", replies[0]);
        let text = replies[0]["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("still running") && text.contains("again"),
            "{text}"
        );
        let report = report_of(&replies[1]);
        let items = report["items"].as_array().unwrap();
        assert!(
            items
                .iter()
                .any(|i| i["category"] == "builds" && i["new"] == false),
            "{report}"
        );
        assert!(
            items
                .iter()
                .any(|i| i["category"] == "worktrees" && i["new"] == true),
            "{report}"
        );
    }

    #[test]
    fn a_dead_baseline_worker_falls_back_to_a_fresh_analysis_on_the_next_call() {
        let (tx, rx) = std::sync::mpsc::channel();
        drop(tx); // the worker is gone without having said anything
        let mut server = Server {
            baseline: Baseline::Pending(rx),
            params: Params::default(),
            deps: Arc::new(deps(one_build(), |_| {})),
        };
        let mut err = Vec::new();
        let reply = server
            .handle(format!("{}", call(1)).as_bytes(), &mut err)
            .unwrap();
        assert_eq!(
            report_of(&reply)["items"][0]["category"],
            "builds",
            "{reply}"
        );
        let err = String::from_utf8_lossy(&err);
        assert!(err.contains("stopped unexpectedly"), "{err}");
    }

    #[test]
    fn eof_while_the_baseline_is_still_blocked_ends_the_server_at_once() {
        let (d, open) = gated_deps(one_build(), Duration::from_secs(30));
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let started = std::time::Instant::now();
        let code = serve(
            Params::default(),
            Cursor::new(Vec::new()),
            &mut out,
            &mut err,
            Arc::new(d),
        );
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        assert!(out.is_empty());
        drop(open);
    }

    #[test]
    fn a_panic_on_the_baseline_worker_falls_back_even_when_another_thread_holds_the_stderr_lock() {
        // The real binary used to hold `stderr().lock()` for its whole life: the panic hook of the
        // worker then waited for that lock before unwinding, the channel never closed and every
        // call waited for the whole bound.
        let _hook = project_panic_hook();
        let _stderr = std::io::stderr().lock();
        let mut d = deps(one_build(), |n| {
            if n == 0 {
                panic!("baseline boom");
            }
        });
        d.baseline_wait = Duration::from_secs(8);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let started = std::time::Instant::now();
        let input = Cursor::new(format!("{}\n", call(1)).into_bytes());
        let code = serve(Params::default(), input, &mut out, &mut err, Arc::new(d));
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
        assert!(
            started.elapsed() < Duration::from_secs(6),
            "{:?}",
            started.elapsed()
        );
        let replies = parse(&out);
        assert_eq!(
            report_of(&replies[0])["items"][0]["category"],
            "builds",
            "{}",
            replies[0]
        );
    }
}
