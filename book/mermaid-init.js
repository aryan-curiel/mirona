// Renders ```mermaid blocks (GitHub renders them natively; mdBook leaves them as code).
(async () => {
  const blocks = document.querySelectorAll("code.language-mermaid");
  if (!blocks.length) return;
  const { default: mermaid } = await import("https://cdn.jsdelivr.net/npm/mermaid@11.4.1/dist/mermaid.esm.min.mjs");
  const dark = ["coal", "navy", "ayu"].some((t) => document.documentElement.classList.contains(t));
  mermaid.initialize({ startOnLoad: false, theme: dark ? "dark" : "default" });
  for (const code of blocks) {
    const div = document.createElement("div");
    div.className = "mermaid";
    div.textContent = code.textContent;
    code.parentElement.replaceWith(div);
  }
  await mermaid.run();
})();
