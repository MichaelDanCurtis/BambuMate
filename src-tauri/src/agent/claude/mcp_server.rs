//! Loopback MCP server that exposes the session's bm_* tools to `claude`.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, JsonObject,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::agent::tools::{ToolContent, ToolOutput, ToolRegistry};
use crate::agent::types::AgentEvent;

pub const PERMISSION_TOOL_NAME: &str = "bm_permission";

pub struct McpHandle {
    pub url: String,
    pub token: String,
    shutdown: Option<oneshot::Sender<()>>,
}

impl McpHandle {
    pub fn mcp_config_json(&self) -> String {
        json!({"mcpServers":{"bambumate":{
            "type":"http",
            "url": self.url,
            "headers":{"Authorization": format!("Bearer {}", self.token)}
        }}})
        .to_string()
    }
}

impl Drop for McpHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

#[derive(Clone)]
struct BmServer {
    registry: Arc<ToolRegistry>,
}

fn schema(v: Value) -> Arc<JsonObject> {
    Arc::new(serde_json::from_value(v).unwrap_or_default())
}

fn to_call_result(out: ToolOutput) -> CallToolResult {
    let content: Vec<ContentBlock> = out
        .content
        .into_iter()
        .map(|c| match c {
            ToolContent::Text(t) => ContentBlock::text(t),
            ToolContent::Image { mime, base64 } => ContentBlock::image(base64, mime),
        })
        .collect();
    if out.ok {
        CallToolResult::success(content)
    } else {
        CallToolResult::error(content)
    }
}

impl BmServer {
    async fn permission(&self, args: &Value) -> CallToolResult {
        let tool = args
            .get("tool_name")
            .and_then(|t| t.as_str())
            .unwrap_or("a tool");
        let input = args.get("input").cloned().unwrap_or_else(|| json!({}));
        let preview: String = input.to_string().chars().take(300).collect();
        let allowed = self
            .registry
            .asks()
            .confirm(
                self.registry.session_id(),
                &format!("Allow Claude Agent to use {tool}? {preview}"),
            )
            .await;
        let decision = if allowed {
            json!({"behavior":"allow","updatedInput": input})
        } else {
            json!({"behavior":"deny","message":"The user declined this action."})
        };
        CallToolResult::success(vec![ContentBlock::text(decision.to_string())])
    }
}

impl ServerHandler for BmServer {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut tools: Vec<Tool> = ToolRegistry::specs()
            .into_iter()
            .map(|s| Tool::new(s.name, s.description, schema(s.input_schema)))
            .collect();
        tools.push(Tool::new(
            PERMISSION_TOOL_NAME,
            "Internal: BambuMate asks the user whether Claude Agent may use a tool.",
            schema(json!({"type":"object","properties":{"tool_name":{"type":"string"},"input":{"type":"object"}},"required":["tool_name","input"]})),
        ));
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let name = request.name.to_string();
        let args = Value::Object(request.arguments.clone().unwrap_or_default());
        if name == PERMISSION_TOOL_NAME {
            return Ok(self.permission(&args).await.into());
        }
        let call_id = uuid::Uuid::new_v4().to_string();
        let sid = self.registry.session_id().to_string();
        let asks = self.registry.asks();
        asks.emit(AgentEvent::ToolCall {
            session_id: sid.clone(),
            call_id: call_id.clone(),
            name: name.clone(),
            args: args.clone(),
        });
        let out = self.registry.call(&name, args).await;
        asks.emit(AgentEvent::ToolResult {
            session_id: sid,
            call_id,
            ok: out.ok,
            summary: out.summary(),
        });
        Ok(to_call_result(out).into())
    }
}

async fn require_token(State(token): State<Arc<String>>, req: Request, next: Next) -> Response {
    let expected = format!("Bearer {token}");
    let ok = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        == Some(expected.as_str());
    if ok {
        next.run(req).await
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

pub async fn start(registry: Arc<ToolRegistry>) -> Result<McpHandle, String> {
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let server = BmServer { registry };
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );
    let router =
        axum::Router::new()
            .nest_service("/mcp", service)
            .layer(middleware::from_fn_with_state(
                Arc::new(token.clone()),
                require_token,
            ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let (tx, rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
    });
    Ok(McpHandle {
        url: format!("http://127.0.0.1:{port}/mcp"),
        token,
        shutdown: Some(tx),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::asks::AskBroker;
    use crate::agent::tools::fake_host::FakeHost;
    use crate::agent::types::AgentEvent;
    use serde_json::json;
    use tokio::sync::broadcast;

    struct Client {
        http: reqwest::Client,
        url: String,
        token: String,
        session: Option<String>,
    }

    impl Client {
        async fn post(&mut self, body: Value, with_token: bool) -> (u16, Option<Value>) {
            let mut req = self
                .http
                .post(&self.url)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream")
                .body(body.to_string());
            if with_token {
                req = req.header("Authorization", format!("Bearer {}", self.token));
            }
            if let Some(s) = &self.session {
                req = req.header("mcp-session-id", s);
            }
            let resp = req.send().await.unwrap();
            let status = resp.status().as_u16();
            if let Some(s) = resp.headers().get("mcp-session-id") {
                self.session = Some(s.to_str().unwrap().to_string());
            }
            let text = resp.text().await.unwrap();
            let json = text
                .lines()
                .filter_map(|l| l.strip_prefix("data: "))
                .filter_map(|d| serde_json::from_str::<Value>(d).ok())
                .find(|v| v.get("id").is_some())
                .or_else(|| serde_json::from_str(&text).ok());
            (status, json)
        }

        async fn connect(handle: &McpHandle) -> Self {
            let mut c = Client {
                http: reqwest::Client::new(),
                url: handle.url.clone(),
                token: handle.token.clone(),
                session: None,
            };
            let (status, init) = c
                .post(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}), true)
                .await;
            assert_eq!(status, 200);
            assert!(init.unwrap()["result"]["capabilities"]["tools"].is_object());
            c.post(
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                true,
            )
            .await;
            c
        }
    }

    fn registry() -> (
        Arc<ToolRegistry>,
        broadcast::Receiver<AgentEvent>,
        Arc<AskBroker>,
    ) {
        let (tx, rx) = broadcast::channel(64);
        let asks = Arc::new(AskBroker::new(tx));
        (
            Arc::new(ToolRegistry::new(
                "s1".into(),
                Arc::new(FakeHost::new()),
                asks.clone(),
            )),
            rx,
            asks,
        )
    }

    #[tokio::test]
    async fn rejects_requests_without_the_token() {
        let (reg, _rx, _asks) = registry();
        let handle = start(reg).await.unwrap();
        let mut c = Client {
            http: reqwest::Client::new(),
            url: handle.url.clone(),
            token: handle.token.clone(),
            session: None,
        };
        let (status, _) = c
            .post(
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
                false,
            )
            .await;
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn lists_registry_tools_plus_permission_tool() {
        let (reg, _rx, _asks) = registry();
        let handle = start(reg).await.unwrap();
        let mut c = Client::connect(&handle).await;
        let (_, list) = c
            .post(
                json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
                true,
            )
            .await;
        let names: Vec<String> = list.unwrap()["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names.len(), 21);
        assert!(names.contains(&"bm_app_state".to_string()));
        assert!(names.contains(&PERMISSION_TOOL_NAME.to_string()));
    }

    #[tokio::test]
    async fn calls_a_tool_and_reports_activity() {
        let (reg, mut rx, _asks) = registry();
        let handle = start(reg).await.unwrap();
        let mut c = Client::connect(&handle).await;
        let (_, res) = c
            .post(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"bm_app_state","arguments":{}}}), true)
            .await;
        let res = res.unwrap();
        assert_eq!(res["result"]["isError"], false);
        assert_eq!(res["result"]["content"][0]["type"], "text");
        assert!(matches!(
            rx.recv().await.unwrap(),
            AgentEvent::ToolCall { .. }
        ));
        assert!(matches!(
            rx.recv().await.unwrap(),
            AgentEvent::ToolResult { ok: true, .. }
        ));
    }

    #[tokio::test]
    async fn permission_tool_asks_the_user_and_returns_claude_decision_json() {
        let (reg, mut rx, asks) = registry();
        let handle = start(reg).await.unwrap();
        let mut c = Client::connect(&handle).await;
        let answerer = tokio::spawn(async move {
            loop {
                if let AgentEvent::Ask { request, .. } = rx.recv().await.unwrap() {
                    assert!(request.question.contains("Bash"));
                    asks.answer(&request.id, vec!["Yes".into()]).unwrap();
                    return;
                }
            }
        });
        let (_, res) = c
            .post(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"bm_permission","arguments":{"tool_name":"Bash","input":{"command":"ls"}}}}), true)
            .await;
        answerer.await.unwrap();
        let text = res.unwrap()["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        let decision: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            decision,
            json!({"behavior":"allow","updatedInput":{"command":"ls"}})
        );
    }

    #[test]
    fn mcp_config_json_carries_url_and_bearer_token() {
        let h = McpHandle {
            url: "http://127.0.0.1:1/mcp".into(),
            token: "tok".into(),
            shutdown: None,
        };
        let v: Value = serde_json::from_str(&h.mcp_config_json()).unwrap();
        assert_eq!(v["mcpServers"]["bambumate"]["type"], "http");
        assert_eq!(
            v["mcpServers"]["bambumate"]["headers"]["Authorization"],
            "Bearer tok"
        );
    }
}
