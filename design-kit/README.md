# Mirona design kit · v0.1

Everything needed to start implementing the Mirona desktop app UI: the interactive prototype, the design system, tokens, logo files and screenshots.

> Mirona: the neighbor on the balcony who hears everything and tells no one. A local-first meeting assistant for people who work with AI agents.

## What's inside

| Folder | Contents |
| --- | --- |
| `prototype/` | `index.html`: the full interactive app prototype. Open it in a browser, no build or server needed. |
| `design-system/` | `index.html`: logo, colors, classification tags, glass surfaces, type scale and core components. |
| `tokens/` | `tokens.css` (CSS variables + ready-made `.mi-*` surface classes) and `tokens.json` (same values, for Tailwind or TS). |
| `brand/` | `app-icon.svg` (64 px and up), `mark-small.svg` (under 64 px), `mark-mono.svg` (uses `currentColor`), `tray-template.svg` (black, for the macOS menu bar), `png/icon-16…1024.png` (full bleed, Windows) and `png/icon-macos-1024.png` (with Apple's margin, macOS). |
| `screenshots/` | Retina screenshots of the current app: `09-settings-window.png` and `10-onboarding-*.png`. The prototype in `prototype/` shows the other screens. |

## Screens in the prototype

1. **Meeting · Transcript**: speaker avatars, timestamps, classification tags, filters (hide small talk, only tagged, show sensitive), speaker-name suggestion.
2. **Meeting · Notes**: your rough notes plus "Enhance with transcript".
3. **Meeting · Summary**: template chosen by classification, model picker (cloud blocked in privacy mode), tickable action items, review chip, quality check.
4. **Inspector panel**: meeting-level answers (type, client with confirm, priority, follow-up + draft), selected segment's typed questions with probability bars against the 0.60 threshold, Correct buttons, owner choice, MCP activity log.
5. **Recording state**: recording bar with timer and levels; live placeholder in the transcript.
6. **Action items**: across meetings, filter by client, tick done.
7. **People and voices**: enrolled voiceprints, enroll a suggested speaker.
8. **Settings window**: its own window, opened from Mirona › Settings… (⌘,), the menu bar icon or the gear in the window chrome (right of the traffic lights on macOS, beside the caption buttons on Windows). Settings has no sidebar item, and the privacy card holds only privacy mode. Sections on the left (General, Privacy and storage, AI models, Clients, Agents (MCP), Advanced, About: version, what Mirona is, credits and "Show the welcome tour"); the section title sits in a fixed title bar and only the content scrolls. The prototype shows AI models: provider per stage (local/cloud), privacy-mode blocking, MCP server status. The MCP activity log is not a setting: it lives in the main window as **Agent activity**. Reference: `screenshots/09-settings-window.png` (`07-models-and-mcp.png` is the old in-window page).

9. **Onboarding** (app only, not in the prototype): its own 980×660 window. On first run it opens before the main window, which appears once it closes; Settings › About replays it on top of Settings, and "Finish setup" in the sidebar opens it at step 2. Four steps: Welcome (what Mirona is and why), Permissions and models, Make it yours (optional choices, saved as you go), You're ready. Everything fits without scrolling. The left panel is a dark teal stage with a slow glow and film grain, and plays on the name (*mirar*, to look; a mirror): each step has its own scene, crossfading between steps. Welcome: the logo's bars speaking over their reflection. Permissions: two sources (mic, speaker) that light up and ripple as each permission is granted. Make it yours: dots orbiting inside a glowing boundary ("on your Mac"); with privacy mode off the boundary turns dashed and a gold dot drifts out. Ready: transcript lines drifting past a reading line, tagged lines in their colour. Steps slide in from the side you're heading to, rows stagger in, and a granted permission pops its check. Reduced motion keeps only the fades. Reference: `screenshots/10-onboarding-welcome.png`, `-permissions`, `-preferences`, `-ready`.

All people, meetings and numbers are sample data.

## Using the tokens in the Tauri + React app

```ts
// main.tsx
import './tokens.css';
```

```tsx
<aside className="mi-panel">…</aside>
<section className="mi-card">…</section>
<div className="mi-row" aria-selected={selected}>…</div>
<span className="mi-chip mi-chip--action">Action · Aryan · Thu</span>
```

Put `mi-backdrop` on the app root so the glass has something to blur. Load Inter (400–700) and JetBrains Mono (400–500), or bundle them for offline use.

For Tailwind, map `tokens.json` into `theme.extend` (colors, borderRadius, spacing, fontFamily).

## App icons

Generate every Tauri icon size from the 1024 PNG:

```bash
pnpm tauri icon brand/png/icon-macos-1024.png   # macOS (keeps Apple's margin)
pnpm tauri icon brand/png/icon-1024.png         # Windows (full bleed)
```

Use `tray-template.svg` (exported to PNG at 22/44 px) as a macOS template image for the tray.

## Notes for implementation

- The prototype uses a tiny offline renderer (`prototype/dc-lite.js`). It's reference only; rebuild the screens as React components.
- Component map: Sidebar · RecordButton · NavItem · MeetingListItem · PrivacySwitch · RecordingBar · MeetingHeader · Tabs · TranscriptSegment · TagChip · SpeakerSuggestion · NotesEditor · EnhancedNotes · SummaryView · ProviderPicker · Inspector · ProbabilityRow · OwnerChoice · AgentLogItem · ActionItemRow · PersonCard · StageRow · McpServerCard · SettingsWindow (SettingsSidebar · SettingsSection).
- Accessibility: text contrast ≥ 4.5:1 on glass (`--mi-muted` is the lightest allowed text color), focus ring on every control, 44 px main actions.
- Respect `prefers-reduced-transparency`: fall back to solid `#F7F8F7` surfaces when it is set.

## Links

- Concept doc: https://claude.ai/artifact/BgtMpUvSgGCRpnXsz3nwwM
- Design canvas: https://claude.ai/artifact/8XJrWGfKNqHTkArdjUkEy2
- Linear project: https://linear.app/cuban-curiel/project/mirona-a6c19874d15b (UI work: NGR-134, NGR-141 onward)
