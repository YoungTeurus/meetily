use crate::gateway::{Gateway, RpcError};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ErrorCode, Implementation,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
        ToolAnnotations,
    },
    service::RequestContext,
    ErrorData, RoleServer, ServerHandler,
};
use serde_json::{json, Map, Value};

#[derive(Clone)]
pub struct MeetilyMcp {
    gateway: Gateway,
    allow_control: bool,
}
impl MeetilyMcp {
    pub fn new(gateway: Gateway, allow_control: bool) -> Self {
        Self {
            allow_control: allow_control && gateway.has_control(),
            gateway,
        }
    }
    pub fn tools(&self) -> Vec<Tool> {
        let page = json!({"cursor":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":1000,"default":20}});
        let mut list = page.as_object().unwrap().clone();
        list.extend(json!({"order":{"type":"string","enum":["asc","desc"],"default":"desc"},"state":{"type":"string"},"query":{"type":"string"}}).as_object().unwrap().clone());
        let mut transcript = page.as_object().unwrap().clone();
        transcript.insert("meeting_id".into(), json!({"type":"string","minLength":1}));
        let mut search = page.as_object().unwrap().clone();
        search.insert("query".into(), json!({"type":"string","minLength":1}));
        let mut tools = vec![
            tool("list_meetings", "List local meetings by date with processing state. Default newest first. Follow next_cursor to retrieve more pages.", list, &[], false),
            tool("get_meeting", "Read meeting metadata including call source, date and processing status.", props(json!({"meeting_id":{"type":"string","minLength":1}})), &["meeting_id"], false),
            tool("get_transcript", "Read ordered timestamped transcript segments; partial indicates processing is incomplete. Follow next_cursor until null; preserve unknown speakers.", transcript, &["meeting_id"], false),
            tool("search_transcripts", "Search local transcripts and return matching snippets with meeting IDs and audio timestamps. Follow next_cursor.", search, &["query"], false),
            tool("export_meeting", "Return a meeting as Markdown, text or JSON content. Does not write files.", props(json!({"meeting_id":{"type":"string","minLength":1},"format":{"type":"string","enum":["md","txt","json"],"default":"md"}})), &["meeting_id"], false),
            tool("get_recording_status", "Read recording lifecycle state and initiator. Optionally select a recording_id; reading never starts recording.", props(json!({"recording_id":{"type":"string","minLength":1}})), &[], false),
            tool("get_detection_status", "Read observed call sessions, confidence, permissions and limitations.", Map::new(), &[], false),
        ];
        if self.allow_control {
            tools.push(tool("start_recording", "Explicitly start microphone and system-audio recording using Meetily's configured speech engine. Repeated calls with the same idempotency_key do not create another recording.", props(json!({"name":{"type":"string"},"language":{"type":"string"},"input_device":{"type":"string"},"output_device":{"type":"string"},"idempotency_key":{"type":"string"}})), &[], true));
            tools.push(tool("stop_recording", "Explicitly stop capture for this recording ID. Transcript finalization may continue; check meeting processing state.", props(json!({"recording_id":{"type":"string","minLength":1}})), &["recording_id"], true));
        }
        tools
    }
}
fn props(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}
fn tool(
    name: &'static str,
    description: &'static str,
    properties: Map<String, Value>,
    required: &[&str],
    control: bool,
) -> Tool {
    let schema = props(
        json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
    );
    Tool::new(name, description, schema).with_annotations(
        ToolAnnotations::new()
            .read_only(!control)
            .destructive(false)
            .idempotent(name != "start_recording")
            .open_world(false),
    )
}

/// Validate the declared schema before dispatch so malformed calls cannot cause
/// gateway-side defaults to turn a misspelled ID into current-recording control.
fn validate(tool: &Tool, args: &Map<String, Value>) -> Result<(), RpcError> {
    let schema = &tool.input_schema;
    let properties = schema.get("properties").and_then(Value::as_object).unwrap();
    for required in schema.get("required").and_then(Value::as_array).unwrap() {
        let key = required.as_str().unwrap();
        if !args.contains_key(key) {
            return Err(RpcError::new(
                "invalid_request",
                format!("Missing argument {key}"),
            ));
        }
    }
    for (key, value) in args {
        let property = properties
            .get(key)
            .ok_or_else(|| RpcError::new("invalid_request", format!("Unknown argument {key}")))?;
        let valid = match property.get("type").and_then(Value::as_str) {
            Some("string") => value.as_str().is_some_and(|s| {
                property
                    .get("minLength")
                    .and_then(Value::as_u64)
                    .is_none_or(|min| s.len() as u64 >= min)
            }),
            Some("integer") => value.as_u64().is_some_and(|n| {
                n >= property.get("minimum").and_then(Value::as_u64).unwrap_or(0)
                    && n <= property
                        .get("maximum")
                        .and_then(Value::as_u64)
                        .unwrap_or(u64::MAX)
            }),
            _ => false,
        };
        let enum_valid = property
            .get("enum")
            .and_then(Value::as_array)
            .is_none_or(|items| items.contains(value));
        if !valid || !enum_valid {
            return Err(RpcError::new(
                "invalid_request",
                format!("Invalid argument {key}"),
            ));
        }
    }
    Ok(())
}
impl ServerHandler for MeetilyMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("meetilyctl", env!("CARGO_PKG_VERSION")))
            .with_instructions("Local Meetily bridge. Use list_meetings ordered by date and ready state, then get_transcript with every page to summarize. Recording is permitted only through separately enabled control tools; connecting never starts it.")
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools().into_iter().find(|tool| tool.name == name)
    }
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if request.and_then(|r| r.cursor).is_some() {
            return Err(ErrorData::invalid_params("Unexpected tools cursor", None));
        }
        Ok(ListToolsResult {
            tools: self.tools(),
            ..Default::default()
        })
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let tool = self.get_tool(&request.name).ok_or_else(|| {
            ErrorData::new(
                ErrorCode::METHOD_NOT_FOUND,
                "Unknown or disabled Meetily tool",
                None,
            )
        })?;
        let mut args = request.arguments.unwrap_or_default();
        if let Err(error) = validate(&tool, &args) {
            return Ok(CallToolResult::structured_error(json!({"error":error})).into());
        }
        let (method, control) = match request.name.as_ref() {
            "list_meetings" => {
                args.entry("order").or_insert(json!("desc"));
                ("meetings.list", false)
            }
            "get_meeting" => ("meetings.get", false),
            "get_transcript" => ("transcript.get", false),
            "search_transcripts" => ("transcripts.search", false),
            "export_meeting" => {
                args.entry("format").or_insert(json!("md"));
                ("meetings.export", false)
            }
            "get_recording_status" => ("status", false),
            "get_detection_status" => ("detection.status", false),
            "start_recording" => {
                args.insert("initiator".into(), json!("mcp"));
                ("recording.start", true)
            }
            "stop_recording" => ("recording.stop", true),
            _ => unreachable!(),
        };
        let result = match self
            .gateway
            .call(method, Value::Object(args), control)
            .await
        {
            Ok(value) => CallToolResult::structured(value),
            Err(error) => CallToolResult::structured_error(json!({"error":error})),
        };
        Ok(result.into())
    }
}
