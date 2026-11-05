# Homebrew formula for the `mirona` CLI (+ `mirona-mcp`), NGR-149.
# Not published yet: the repo is private and has no tagged releases. To publish, copy this file
# into a tap repo (nagare-studio/homebrew-tap → `brew install nagare-studio/tap/mirona`) and add
# a stable `url` + `sha256` for a release tarball. Until then, from a checkout:
#   brew install --HEAD --formula ./packaging/homebrew/mirona.rb
# The build downloads sherpa-onnx's prebuilt static libraries from GitHub (sherpa-onnx-sys).
class Mirona < Formula
  desc "Local-first meeting memory: record, transcribe, summarize, MCP server"
  homepage "https://github.com/nagare-studio/mirona"
  license "Apache-2.0"
  head "https://github.com/nagare-studio/mirona.git", branch: "main"

  depends_on "rust" => :build
  depends_on :macos # live capture is macOS-only in M1

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/mirona-cli")
    system "cargo", "install", *std_cargo_args(path: "crates/mirona-mcp")
  end

  test do
    (testpath/"app/config.toml").write "meetings_dir = \"#{testpath}/meetings\"\n"
    ENV["MIRONA_APP_DIR"] = testpath/"app"
    assert_match "Indexed 0 meetings", shell_output("#{bin}/mirona reindex")
  end
end
