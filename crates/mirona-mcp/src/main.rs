//! `mirona-mcp`: Mirona's MCP server on stdio (NGR-148). Same as `mirona mcp`.

fn main() {
    if let Err(e) = mirona_mcp::serve() {
        eprintln!("mirona-mcp: {e}");
        std::process::exit(1);
    }
}
