//! `mirona-mcp`: Mirona's MCP server on stdio (NGR-148). Same as `mirona mcp`.
//!
//! A thin wrapper over [`mirona_mcp::serve`], so MCP clients that spawn a command (Claude
//! Code, Claude Desktop) have a binary of their own. It exits with an error while MCP is
//! turned off in config. The tools, the HTTP transport and the activity log live in the
//! `mirona_mcp` library.

fn main() {
    if let Err(e) = mirona_mcp::serve() {
        eprintln!("mirona-mcp: {e}");
        std::process::exit(1);
    }
}
