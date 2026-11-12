//! The server: one tool and one resource. Replace them with yours; keep the rules in the test.
//!
//! Every tool needs a description, an annotation that says whether it only reads or can change
//! things, and inputs with a bound (a maximum length, a range). tests/server.rs fails when one is
//! missing, so a tool a host will let a model call is never left unlabelled.
use rmcp::{
    ErrorData, Json, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        ListResourcesResult, PaginatedRequestParams, ReadResourceRequestParams,
        ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, ServerCapabilities,
        ServerConfig,
    },
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_TEXT: usize = 10_000;
const ABOUT: &str = "@@NAME@@: counts words. Read-only; it holds no credentials and calls nothing outside this process.";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CountInput {
    /// The text to count.
    #[schemars(length(max = 10_000))]
    pub text: String,
}

/// The structured result: a host gets it as structuredContent, checked against the output schema.
#[derive(Serialize, JsonSchema)]
pub struct Count {
    pub words: usize,
    pub characters: usize,
}

#[derive(Clone)]
pub struct Server {
    tool_router: ToolRouter<Self>,
}

impl Server {
    pub fn new() -> Self {
        Self { tool_router: Self::tool_router() }
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router]
impl Server {
    #[tool(
        name = "word_count",
        description = "Count the words and the characters in a text (at most 10,000 characters).",
        annotations(
            read_only_hint = true,
            idempotent_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn word_count(&self, Parameters(input): Parameters<CountInput>) -> Result<Json<Count>, String> {
        if input.text.chars().count() > MAX_TEXT {
            return Err(format!("text is longer than {MAX_TEXT} characters"));
        }
        Ok(Json(Count {
            words: input.text.split_whitespace().count(),
            characters: input.text.chars().count(),
        }))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_resources().build())
            .with_server_info(rmcp::model::Implementation::new("@@NAME@@", "0.1.0"))
            .with_instructions("Counts the words and characters in a text. It reads nothing else and changes nothing.")
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let about = Resource::new("about://server", "about")
            .with_description("What this server is and what it can do.")
            .with_mime_type("text/plain");
        Ok(ListResourcesResult::with_all_items(vec![about]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        if request.uri != "about://server" {
            return Err(ErrorData::resource_not_found("no such resource", None));
        }
        Ok(ReadResourceResult::new(vec![ResourceContents::text(ABOUT, request.uri)]).into())
    }
}
