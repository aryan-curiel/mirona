import { useEffect, useRef, useState } from "react";
import { useStore } from "../store";
import { allTags, api, dayGroup, fmt, typeLabel, type McpActivity, type Person } from "../lib/api";
import { Avatar, Icon, PageHeader } from "../components/ui";
import { tagClass } from "../lib/match";


type Status = "all" | "open" | "done" | "review";
const STATUS: [Status, string][] = [
  ["all", "All"],
  ["open", "Open"],
  ["done", "Done"],
  ["review", "Needs review"],
];

function Pills<T extends string | null>({ value, options, onPick, label }: { value: T; options: [T, string][]; onPick: (v: T) => void; label: string }) {
  return (
    <fieldset className="flex flex-wrap gap-2" aria-label={label}>
      {options.map(([v, name]) => (
        <button
          type="button"
          key={v ?? "all"}
          aria-pressed={value === v}
          onClick={() => onPick(v)}
          className={`min-h-9 rounded-full px-4 text-[14px] font-medium ${value === v ? "bg-ink text-white" : "mi-ghost rounded-full!"}`}
        >
          {name}
        </button>
      ))}
    </fieldset>
  );
}

type Sort = "date" | "title" | "tags" | "length";

/** Every meeting in one sortable table, with bulk delete (the sidebar lists only the newest). */
export function AllMeetingsView() {
  const s = useStore();
  const [q, setQ] = useState("");
  const [tag, setTag] = useState("");
  const [type, setType] = useState("");
  const [sort, setSort] = useState<{ by: Sort; asc: boolean }>({ by: "date", asc: false });
  const [picked, setPicked] = useState<string[]>([]);
  const [sure, setSure] = useState<"picked" | "samples" | null>(null);
  const tags = allTags(null, s.meetings);
  const types = [...new Set(s.meetings.map((m) => m.meetingType).filter(Boolean))].sort();
  const samples = s.meetings.filter((m) => m.sample).map((m) => m.id);
  const needle = q.trim().toLowerCase();
  const key = (m: (typeof s.meetings)[number]) =>
    sort.by === "date" ? m.created : sort.by === "title" ? m.title.toLowerCase() : sort.by === "tags" ? m.tags.join(" ").toLowerCase() : m.durationMin;
  const rows = s.meetings
    .filter((m) => (!needle || m.title.toLowerCase().includes(needle)) && (!tag || m.tags.includes(tag)) && (!type || m.meetingType === type))
    .sort((a, b) => {
      const [x, y] = [key(a), key(b)];
      return (x < y ? -1 : x > y ? 1 : 0) * (sort.asc ? 1 : -1);
    });
  const visible = rows.map((m) => m.id);
  const pickedHere = picked.filter((id) => visible.includes(id));
  const allPicked = rows.length > 0 && pickedHere.length === rows.length;
  const flip = (id: string) => setPicked(picked.includes(id) ? picked.filter((x) => x !== id) : [...picked, id]);
  const del = async (which: "picked" | "samples") => {
    if (sure !== which) return setSure(which);
    setSure(null);
    await s.deleteMeetings(which === "picked" ? pickedHere : samples);
    setPicked([]);
  };
  // A plain function, not a component, so sorting doesn't remount the header (and lose focus).
  const head = (by: Sort, label: string, className = "") => (
    <th className={`px-3 py-2.5 text-left font-medium ${className}`} aria-sort={sort.by === by ? (sort.asc ? "ascending" : "descending") : undefined}>
      <button type="button" className="rounded hover:text-ink" onClick={() => setSort({ by, asc: sort.by === by ? !sort.asc : by !== "date" })}>
        {label} {sort.by === by ? (sort.asc ? "↑" : "↓") : ""}
      </button>
    </th>
  );
  let lastGroup = "";
  return (
    <div className="mx-auto max-w-[1100px] px-10 pt-8 pb-16">
      <PageHeader title="All meetings">
        {s.meetings.length} meeting{s.meetings.length === 1 ? "" : "s"} in <span className="font-mono text-[13px]">{s.settings?.meetingsDir}</span>.
      </PageHeader>
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <input type="search" className="mi-field w-64 text-[14px]" placeholder="Filter by title" aria-label="Filter by title" value={q} onChange={(e) => setQ(e.target.value)} />
        <select className="mi-field text-[14px]" aria-label="Tag" value={tag} onChange={(e) => setTag(e.target.value)}>
          <option value="">All tags</option>
          {tags.map((t) => (
            <option key={t}>{t}</option>
          ))}
        </select>
        {types.length > 0 && (
          <select className="mi-field text-[14px]" aria-label="Meeting type" value={type} onChange={(e) => setType(e.target.value)}>
            <option value="">All types</option>
            {types.map((t) => (
              <option key={t} value={t}>
                {typeLabel(t)}
              </option>
            ))}
          </select>
        )}
        <span className="flex-1" />
        {samples.length > 0 && (
          <button type="button" className={`mi-ghost text-[13px] ${sure === "samples" ? "text-record-text!" : ""}`} onBlur={() => setSure(null)} onClick={() => void del("samples")}>
            {sure === "samples" ? `Delete ${samples.length} demo meetings for good` : "Remove demo meetings"}
          </button>
        )}
        {pickedHere.length > 0 && (
          <button type="button" className={`mi-ghost text-[13px] ${sure === "picked" ? "text-record-text!" : ""}`} onBlur={() => setSure(null)} onClick={() => void del("picked")}>
            {sure === "picked" ? `Delete ${pickedHere.length} for good, everywhere` : `Delete ${pickedHere.length} selected…`}
          </button>
        )}
      </div>
      <div className="mi-card overflow-x-auto">
        <table className="w-full text-[14px]">
          <thead className="border-b border-line text-[13px] text-muted">
            <tr>
              <th className="w-10 px-3 py-2.5">
                <input type="checkbox" aria-label="Select all shown" checked={allPicked} onChange={() => setPicked(allPicked ? picked.filter((id) => !visible.includes(id)) : [...new Set([...picked, ...visible])])} />
              </th>
              {head("title", "Title")}
              {head("tags", "Tags")}
              <th className="px-3 py-2.5 text-left font-medium">Type</th>
              {head("date", "Date")}
              {head("length", "Length", "text-right")}
              <th className="px-3 py-2.5 text-left font-medium">Status</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((m) => {
              const g = sort.by === "date" ? dayGroup(m.created) : "";
              const header = g !== lastGroup;
              lastGroup = g;
              return [
                header && g && (
                  <tr key={`g-${g}`}>
                    <td colSpan={7} className="px-3 pt-4 pb-1 text-[12px] font-semibold text-muted">
                      {g}
                    </td>
                  </tr>
                ),
                <tr key={m.id} className="border-t border-line-soft hover:bg-white/40">
                  <td className="px-3 py-2.5">
                    <input type="checkbox" aria-label={`Select ${m.title}`} checked={picked.includes(m.id)} onChange={() => flip(m.id)} />
                  </td>
                  <td className="max-w-[320px] px-3 py-2.5">
                    <button type="button" className="truncate text-left font-semibold hover:text-accent-strong hover:underline" onClick={() => s.openMeeting(m.id)}>
                      {m.title}
                    </button>
                    {m.sample && <span className="ml-2 text-[12px] text-muted">Sample</span>}
                  </td>
                  <td className="max-w-[220px] px-3 py-2.5 text-ink-3">
                    {m.tags.length ? (
                      <span className="flex flex-wrap gap-1">
                        {m.tags.map((t) => (
                          <span key={t} className={`mi-chip ${tagClass(t)}`}>
                            {t}
                          </span>
                        ))}
                      </span>
                    ) : (
                      "—"
                    )}
                  </td>
                  <td className="px-3 py-2.5 text-ink-3">{m.meetingType ? typeLabel(m.meetingType) : "—"}</td>
                  <td className="px-3 py-2.5 whitespace-nowrap text-ink-3">{m.when}</td>
                  <td className="px-3 py-2.5 text-right whitespace-nowrap text-ink-3">{m.durationMin} min</td>
                  <td className="px-3 py-2.5">
                    <span className="flex flex-wrap gap-1">
                      {m.openActions > 0 && <span className="mi-chip mi-chip--action">{m.openActions} open</span>}
                      {m.needsReview > 0 && <span className="mi-chip mi-chip--review">{m.needsReview} to check</span>}
                      {!m.hasSummary && <span className="mi-chip mi-chip--neutral">No summary</span>}
                      {!m.hasAudio && <span className="mi-chip mi-chip--neutral">No audio</span>}
                    </span>
                  </td>
                </tr>,
              ];
            })}
          </tbody>
        </table>
        {!rows.length && <p className="px-6 py-10 text-center text-[14px] text-muted">{s.meetings.length ? "No meetings match these filters." : "No meetings yet. Start a recording or import an audio file."}</p>}
      </div>
    </div>
  );
}

/** Across meetings, filtered like MCP's `list_action_items`: a meeting tag, "Open" is
 * `open_only`; "Needs review" lists the unconfirmed ones (NGR-161). */
export function ActionItemsView() {
  const s = useStore();
  const [tag, setTag] = useState("");
  const [status, setStatus] = useState<Status>("all");
  const tags = allTags(null, s.actionItems);
  const rows = s.actionItems.filter(
    (a) =>
      (!tag || a.tags.includes(tag)) &&
      (status === "all" || (status === "open" && !a.done) || (status === "done" && a.done) || (status === "review" && a.needsReview)),
  );
  return (
    <div className="mx-auto max-w-[1100px] px-10 pt-8 pb-16">
      <PageHeader title="Action items">
        Detected by classification across meetings. Agents read the same list through <code className="font-mono text-[13px]">list_action_items</code>; items marked
        “needs review” reach them as unconfirmed.
      </PageHeader>
      <div className="mb-3 flex flex-wrap items-center gap-x-6 gap-y-3">
        <select className="mi-field text-[14px]" aria-label="Filter by tag" value={tag} onChange={(e) => setTag(e.target.value)}>
          <option value="">All tags</option>
          {tags.map((t) => (
            <option key={t}>{t}</option>
          ))}
        </select>
        <Pills label="Filter by status" value={status} options={STATUS} onPick={setStatus} />
      </div>
      <p className="mb-3 text-[13px] text-muted" aria-live="polite">
        {rows.length} item{rows.length === 1 ? "" : "s"}
      </p>
      <div className="mi-card overflow-hidden">
        {rows.map((a, i) => (
          <div key={a.id} style={{ animationDelay: `${Math.min(i, 8) * 30}ms` }} className={`mi-enter grid grid-cols-[auto_1fr_120px_110px_auto] items-center gap-4 px-6 py-4 ${i ? "border-t border-line-soft" : ""}`}>
            <input type="checkbox" checked={a.done} onChange={() => s.toggleAction(a.id)} aria-label={`Done: ${a.text}`} />
            <div className="min-w-0">
              <p className={`text-[15px] ${a.done ? "text-muted line-through" : ""}`}>
                {a.text} {a.needsReview && <span className="mi-chip mi-chip--review ml-1">Needs review</span>}
              </p>
              <button
                type="button"
                className="text-left text-[13px] text-muted hover:text-accent hover:underline"
                aria-label={`Open ${a.meetingTitle}${a.atMs != null ? ` at ${fmt(a.atMs)}` : ""}`}
                onClick={() => (a.atMs != null ? s.openAt(a.meetingId, a.atMs) : s.openMeeting(a.meetingId))}
              >
                {a.meetingTitle}
                {a.atMs != null && ` · ${fmt(a.atMs)}`}
              </button>
            </div>
            <span className="truncate text-[15px]" title={a.ownerConfirmed ? "Owner set by you" : undefined}>
              {a.owner || <span className="text-muted">No owner</span>}
            </span>
            <span className="truncate text-[13px] text-ink-3">{a.due ?? "—"}</span>
            <span className="flex flex-wrap justify-end gap-1 justify-self-end">
              {a.tags.length ? (
                a.tags.map((t) => (
                  <span key={t} className={`mi-chip ${tagClass(t)}`}>
                    {t}
                  </span>
                ))
              ) : (
                <span className="mi-chip mi-chip--neutral">No tag</span>
              )}
            </span>
          </div>
        ))}
        {!rows.length && <p className="px-6 py-10 text-center text-[14px] text-muted">{s.actionItems.length ? "No action items match these filters." : "No action items yet. They come from each meeting's summary."}</p>}
      </div>
    </div>
  );
}

/** Demo people have voiceprint labels but no voice data: a fixed pattern from their id stands in, so
 * the ring can be seen. Never stored, so it can't match anyone in a real recording. */
function demoShape(id: string): number[] {
  let h = [...id].reduce((a, ch) => (Math.imul(a, 31) + ch.charCodeAt(0)) >>> 0, 7);
  const rand = () => {
    h = (Math.imul(h, 1664525) + 1013904223) >>> 0;
    return h / 2 ** 32;
  };
  const phase = rand() * 6;
  const raw = Array.from({ length: 48 }, (_, i) => Math.abs(Math.sin(i * (0.25 + rand() * 0.02) + phase)) * 0.6 + rand() * 0.4);
  const lo = Math.min(...raw);
  const hi = Math.max(...raw);
  return raw.map((x) => (x - lo) / (hi - lo || 1));
}

/** The avatar inside a ring of bars drawn from the person's voiceprint: every voice gets its own ring. */
function VoiceRing({ p, size = 64 }: { p: Person; size?: number }) {
  const [shape, setShape] = useState<number[] | null>(null);
  const enrolled = p.voiceprint?.meetings ?? 0;
  const demo = !!p.lastSeen?.sample;
  useEffect(() => {
    if (!enrolled) return setShape(null);
    api.voiceprintShape(p.id).then(
      (real) => setShape(real ?? (demo ? demoShape(p.id) : null)),
      () => setShape(null),
    );
  }, [p.id, enrolled, demo]);
  const c = size / 2;
  const inner = size * 0.36;
  return (
    <span className="relative grid shrink-0 place-items-center" style={{ width: size, height: size }}>
      {shape && (
        <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className="absolute inset-0 text-accent" aria-hidden="true">
          {shape.map((v, i) => {
            const a = (i / shape.length) * Math.PI * 2 - Math.PI / 2;
            const r2 = inner + 3 + v * (c - inner - 4);
            return (
              <line
                // biome-ignore lint/suspicious/noArrayIndexKey: fixed-length shape, one bar per slice
                key={i}
                x1={c + Math.cos(a) * (inner + 2)}
                y1={c + Math.sin(a) * (inner + 2)}
                x2={c + Math.cos(a) * r2}
                y2={c + Math.sin(a) * r2}
                stroke="currentColor"
                strokeWidth={1.6}
                strokeLinecap="round"
                opacity={0.35 + v * 0.65}
              />
            );
          })}
        </svg>
      )}
      <Avatar tint={p.tint} initial={p.initial} size={Math.round(inner * 2 - 2)} />
    </span>
  );
}

/** "Same person as…": everyone else, with a quick search. Native <dialog> keeps focus inside and
 * closes on Escape; picking someone still needs the Merge button. */
function MergeDialog({ p, others, onClose }: { p: Person; others: Person[]; onClose: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const merge = useStore((s) => s.merge);
  const [q, setQ] = useState("");
  const [pick, setPick] = useState<string | null>(null);
  useEffect(() => ref.current?.showModal(), []);
  const needle = q.trim().toLowerCase();
  const list = others.filter((o) => !needle || [o.name, o.subtitle, o.lastSeen?.title].some((t) => t?.toLowerCase().includes(needle)));
  const chosen = others.find((o) => o.id === pick);
  const close = () => ref.current?.close();
  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: a click on the backdrop closes; the keyboard has Escape (native to <dialog>)
    <dialog
      ref={ref}
      aria-labelledby={`merge-${p.id}`}
      onClose={onClose}
      onClick={(e) => e.target === ref.current && close()}
      className="mi-card mi-pop m-auto w-[min(460px,calc(100vw-48px))] text-ink backdrop:bg-black/20"
    >
      <div className="flex max-h-[70vh] flex-col p-5">
        <h2 id={`merge-${p.id}`} className="text-[17px] font-semibold">
          Who is {p.name} really?
        </h2>
        <p className="mt-1 text-[13px] text-ink-3">{p.name} becomes the person you pick in every meeting, and their voice joins that voiceprint.</p>
        <input
          autoFocus
          type="search"
          className="mi-field mt-4 text-[14px]"
          placeholder="Search people or meetings"
          aria-label="Search people"
          value={q}
          onChange={(e) => {
            setQ(e.target.value);
            setPick(null);
          }}
          onKeyDown={(e) => e.key === "Enter" && list.length === 1 && setPick(list[0].id)}
        />
        <div className="-mx-2 mt-3 min-h-0 flex-1 overflow-y-auto">
          {list.length === 0 && <p className="px-2 py-3 text-[13px] text-muted">Nobody matches “{q.trim()}”.</p>}
          {list.map((o) => (
            <button
              type="button"
              key={o.id}
              aria-pressed={pick === o.id}
              onClick={() => setPick(o.id)}
              className="mi-nav flex w-full items-center gap-3 px-2 py-2 text-left"
            >
              <Avatar tint={o.tint} initial={o.initial} size={32} />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[14px] font-semibold">
                  {o.name}
                  {o.subtitle && <span className="ml-2 text-[12px] font-normal text-muted">{o.subtitle}</span>}
                </span>
                {o.lastSeen && <span className="block truncate text-[12px] text-ink-3">{o.lastSeen.when} in {o.lastSeen.title}</span>}
              </span>
              {pick === o.id && <Icon name="tick" className="shrink-0 text-accent" />}
            </button>
          ))}
        </div>
        <div className="mt-4 flex items-center justify-end gap-2">
          <button type="button" className="mi-ghost text-[14px]" onClick={close}>
            Cancel
          </button>
          <button type="button" className="mi-btn-primary h-10 px-4 text-[14px] disabled:opacity-50" disabled={!chosen} onClick={() => chosen && merge(p.id, chosen.id).then(close)}>
            {chosen ? `Merge into ${chosen.name}` : "Merge"}
          </button>
        </div>
      </div>
    </dialog>
  );
}

/** The user: the mic speaker ("me" in recordings) or the one the demo labels "You".
 * ponytail: guessed from id and subtitle; a real "this is me" flag on Speaker if enrolment renames break it. */
const isMe = (p: Person) => p.id === "me" || p.subtitle === "You";

function PersonCard({ p }: { p: Person }) {
  const s = useStore();
  const [name, setName] = useState<string | null>(null);
  const [merging, setMerging] = useState(false);
  const [sure, setSure] = useState(false);
  const others = s.people.filter((o) => o.id !== p.id);
  const vp = p.voiceprint?.meetings ?? 0;
  return (
    <section
      className={`mi-card mi-enter flex flex-col gap-3 p-4 ${isMe(p) ? "border-accent! bg-[var(--mi-accent-tint)]! ring-1 ring-accent" : ""}`}
      aria-label={isMe(p) ? `${p.name} (you)` : p.name}
    >
      <div className="flex items-center gap-3">
        <VoiceRing p={p} />
        <div className="min-w-0 flex-1">
          {name === null ? (
            <div className="flex min-w-0 items-center gap-1">
              <p className="min-w-0 truncate text-[16px] font-semibold" title={p.name}>
                {p.name}
                {isMe(p) ? (
                  <span className="mi-chip ml-2 bg-accent align-middle text-white">You</span>
                ) : (
                  p.subtitle && <span className="ml-2 text-[13px] font-normal text-muted">{p.subtitle}</span>
                )}
              </p>
              <button type="button" aria-label={`Rename ${p.name}`} title="Rename" className="grid size-7 shrink-0 place-items-center rounded-md text-muted hover:bg-white/60 hover:text-ink" onClick={() => setName(p.name)}>
                <Icon name="pencil" size={15} />
              </button>
            </div>
          ) : (
            <form
              className="flex gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                if (name.trim() && name.trim() !== p.name) void s.enroll(p.id, name.trim());
                setName(null);
              }}
            >
              {/* biome-ignore lint/a11y/noAutofocus: opened by the user's Rename click */}
              <input autoFocus className="mi-field h-8 min-w-0 flex-1 text-[14px]" aria-label={`New name for ${p.name}`} value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === "Escape" && setName(null)} />
              <button type="submit" className="mi-ghost text-[13px]">Save</button>
            </form>
          )}
          <p className="truncate text-[13px] text-ink-3" title={p.lastSeen ? `Last seen ${p.lastSeen.when} in ${p.lastSeen.title}` : undefined}>
            {p.meetings} meeting{p.meetings === 1 ? "" : "s"}
            {p.lastSeen && ` · ${p.lastSeen.when} in ${p.lastSeen.title}`}
          </p>
          {vp ? (
            <p className="flex items-center gap-1 text-[12px] text-accent-strong">
              <span className="truncate">
                Voiceprint from {vp} meeting{vp === 1 ? "" : "s"}
              </span>
              <span aria-hidden="true">·</span>
              <button
                type="button"
                className="shrink-0 whitespace-nowrap rounded font-medium text-record-text hover:underline"
                aria-label={sure ? `Click again to delete ${p.name}'s voiceprint for good` : `Delete ${p.name}'s voiceprint`}
                onBlur={() => setSure(false)}
                onClick={() => (sure ? s.deleteVoiceprint(p.id).then(() => setSure(false)) : setSure(true))}
              >
                {sure ? "Delete for good?" : "Delete"}
              </button>
            </p>
          ) : (
            <p className="truncate text-[12px] text-muted" title="Not recognized in new meetings until a voiceprint is enrolled">
              No voiceprint yet
            </p>
          )}
        </div>
      </div>
      {p.suggestion && (
        <div className="flex items-center gap-2 rounded-md border border-dashed border-[var(--mi-review-edge)] bg-[var(--mi-review-tint)] px-3 py-2 text-[13px]">
          <span className="min-w-0 flex-1 truncate">
            Sounds like <b>{p.suggestion.name}</b> · {p.suggestion.probability.toFixed(2)}
          </span>
          <button type="button" className="mi-ghost shrink-0 text-[13px]" onClick={() => p.suggestion && s.enroll(p.id, p.suggestion.name)}>
            Enroll
          </button>
        </div>
      )}
      {/* Merging is rare: a link that opens a searchable list of everyone else. */}
      {others.length > 0 && (
        <button type="button" className="-mx-1 mt-auto self-start rounded px-1 text-[13px] font-medium text-accent-strong hover:underline" onClick={() => setMerging(true)}>
          Same person as…
        </button>
      )}
      {merging && <MergeDialog p={p} others={others} onClose={() => setMerging(false)} />}
    </section>
  );
}

export function PeopleView() {
  const people = useStore((s) => s.people);
  return (
    <div className="mx-auto max-w-[1100px] px-10 pt-8 pb-16">
      <PageHeader title="People and voices">
        Diarization splits each call by voice; enrolled voiceprints turn “Speaker 2” into a name across meetings. Voiceprints never leave this device. Renaming
        updates every transcript, summary and action item.
      </PageHeader>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(300px,1fr))] gap-4">
        {/* You first, everyone else in the order the store gives. */}
        {[...people.filter(isMe), ...people.filter((p) => !isMe(p))].map((p) => (
          <PersonCard key={p.id} p={p} />
        ))}
      </div>
      {!people.length && <p className="text-[14px] text-muted">Nobody yet. Speakers show up here after a recording.</p>}
    </div>
  );
}

/** The newest MCP tool calls: which agent called what. Never the arguments or results. */
export function AgentActivityView() {
  const [log, setLog] = useState<McpActivity[]>([]);
  useEffect(() => {
    const load = () => api.mcpActivity(20).then(setLog, () => {});
    load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  }, []);
  return (
    <div className="mx-auto max-w-[1100px] px-10 pt-8 pb-16">
      <PageHeader title="Agent activity">The newest MCP tool calls: which agent called what. Arguments and results are never logged.</PageHeader>
      <section className="mi-card p-6">
      {log.length === 0 ? (
        <p className="text-[14px] text-ink-3">No agent has called Mirona yet. Connect one in Settings › Agents (MCP).</p>
      ) : (
        <ul className="flex flex-col gap-1 text-[13px]">
          {log.map((a, i) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: entries have no id; the list is replaced whole
            <li key={i} className="flex gap-3">
              <span className="w-40 shrink-0 text-muted">{new Date(a.time).toLocaleString()}</span>
              <span className="w-40 shrink-0 truncate">{a.client}</span>
              <span className="flex-1 font-mono">{a.tool}</span>
              <span className={a.ok ? "text-accent-strong" : "text-record-text"}>{a.ok ? "ok" : "error"}</span>
            </li>
          ))}
        </ul>
      )}
      </section>
    </div>
  );
}
