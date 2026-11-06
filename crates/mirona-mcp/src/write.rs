//! Write tools (NGR-168), only listed and callable while `mcp_write` is on (see
//! `Server::router`). They change the meeting files the way the app does; the app picks the
//! change up through `.mirona/generation`.

use crate::Server;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{schemars, tool, tool_router};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct DoneArgs {
    /// Action item id, as returned by `list_action_items` or `get_meeting`.
    pub id: String,
    /// true: done; false: open again.
    pub done: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct RenameArgs {
    /// Meeting id, as returned by `search_meetings`.
    pub meeting_id: String,
    /// The speaker's id in that meeting, from `get_meeting`'s `speakers`.
    pub speaker_id: String,
    /// Who it really is. A name a known person already has merges this voice into them.
    pub name: String,
}

#[tool_router(router = write_router, vis = "pub(crate)")]
impl Server {
    #[tool(
        description = "Mark an action item done (or open again). Changes the user's meeting \
        files: only when the user asked for it.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    fn set_action_item_done(&self, Parameters(a): Parameters<DoneArgs>) -> Result<String, String> {
        let mut store = self.store()?;
        let item = store
            .action_items()
            .into_iter()
            .find(|i| i.id == a.id)
            .ok_or_else(|| format!("no action item {}", a.id))?;
        let item = if item.done == a.done {
            item
        } else {
            store.toggle_action_item(&a.id)?
        };
        Ok(json!({ "id": item.id, "text": item.text, "done": item.done }).to_string())
    }

    #[tool(
        description = "Name a speaker in a meeting (e.g. \"them\" → \"Karen\"). The name \
        follows the voice to its other meetings. Changes the user's meeting files: only when \
        the user asked for it.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    fn rename_speaker(&self, Parameters(a): Parameters<RenameArgs>) -> Result<String, String> {
        let mut store = self.store()?;
        let s = store.name_speaker(&a.meeting_id, &a.speaker_id, &a.name)?;
        Ok(json!({ "id": s.id, "name": s.name }).to_string())
    }
}
