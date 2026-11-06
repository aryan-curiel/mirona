import { type MouseEvent, type ReactNode, useRef } from "react";
import type { TagKind } from "../bindings/TagKind";
import type { Tint } from "../bindings/Tint";

/** Icons from the design kit (24px grid, stroke = currentColor). */
const paths = {
  mic: <><rect x="9" y="3" width="6" height="11" rx="3" /><path d="M5 11a7 7 0 0 0 14 0M12 18v3" /></>,
  list: <path d="M4 6h16M4 12h16M4 18h10" />,
  check: <><rect x="3" y="3" width="18" height="18" rx="4" /><path d="M8 12l3 3 5-6" /></>,
  people: <><circle cx="9" cy="8" r="3.5" /><path d="M2.5 20c.8-3.5 3.4-5.5 6.5-5.5s5.7 2 6.5 5.5M16 4.5a3.5 3.5 0 0 1 0 7M18 14.7c1.8.7 3 2.6 3.5 5.3" /></>,
  sliders: <><path d="M4 7h10M18 7h2M4 17h4M12 17h8" /><circle cx="16" cy="7" r="2" /><circle cx="10" cy="17" r="2" /></>,
  shield: <path d="M12 3l8 3v6c0 4.5-3.4 8-8 9-4.6-1-8-4.5-8-9V6z" />,
  tick: <path d="M5 12l5 5 9-10" />,
  gear: <><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" /></>,
  pencil: <path d="M4 20h4L18.5 9.5a2.1 2.1 0 0 0-4-4L4 16v4zM13.5 6.5l4 4" />,
  plug: <path d="M9 7V3M15 7V3M7 7h10v4a5 5 0 0 1-10 0zM12 16v5" />,
};

export type IconName = keyof typeof paths;

export function Icon({ name, size = 18, className = "" }: { name: IconName; size?: number; className?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" className={className}>
      {paths[name]}
    </svg>
  );
}

export function Logo({ size = 30 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 100 100" aria-hidden="true">
      <rect width="100" height="100" rx="24" fill="#0E7466" />
      <path d="M28 72V46M39 72V32M50 72V26M61 72V36M72 72V48" stroke="#fff" strokeWidth="7" strokeLinecap="round" />
      <rect x="16" y="72" width="68" height="8" rx="4" fill="#fff" />
    </svg>
  );
}

const chipClass: Record<TagKind | "neutral", string> = {
  neutral: "mi-chip--neutral",
  action: "mi-chip--action",
  decision: "mi-chip--decision",
  openQuestion: "mi-chip--question",
  smallTalk: "mi-chip--neutral",
  sensitive: "mi-chip--sensitive",
  review: "mi-chip--review",
};

export function Chip({ kind, children, onClick, label }: { kind: TagKind | "neutral"; children: ReactNode; onClick?: (e: MouseEvent) => void; label?: string }) {
  const cls = `mi-chip ${chipClass[kind]}`;
  return onClick ? (
    <button type="button" className={cls} onClick={onClick} aria-label={label}>
      {children}
    </button>
  ) : (
    <span className={cls}>{children}</span>
  );
}

const tintClass: Record<Tint, string> = {
  accent: "bg-[var(--mi-accent-avatar)] text-accent-strong",
  indigo: "bg-[var(--mi-avatar-indigo)] text-decision",
  amber: "bg-[var(--mi-avatar-amber)] text-question",
};

export function Avatar({ tint, initial, size = 36 }: { tint: Tint; initial: string; size?: number }) {
  return (
    <span className={`grid shrink-0 place-items-center rounded-full font-semibold ${tintClass[tint]}`} style={{ width: size, height: size, fontSize: size * 0.42 }}>
      {initial}
    </span>
  );
}

export function Toggle({ on, onChange, label }: { on: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <button type="button" role="switch" aria-checked={on} aria-label={label} className="mi-toggle" onClick={() => onChange(!on)}>
      <i />
    </button>
  );
}

export function Label({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <div className={`mi-label ${className}`}>{children}</div>;
}

export function Mono({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <span className={`font-mono text-[12px] text-muted ${className}`}>{children}</span>;
}

export function PageHeader({ title, children }: { title: string; children: ReactNode }) {
  return (
    <header className="mb-5">
      <h1 className="text-[26px] font-bold leading-tight tracking-[-0.015em]">{title}</h1>
      <p className="mt-1.5 text-[14px] text-muted">{children}</p>
    </header>
  );
}

/** The last non-null value, so a closing panel can keep showing its content while it animates out. */
export function useLast<T>(value: T | null): T | null {
  const last = useRef(value);
  if (value) last.current = value;
  return last.current;
}

/** Slides its content open and closed by height, so what sits below glides instead of jumping. */
export function Reveal({ open, children }: { open: boolean; children: ReactNode }) {
  return (
    <div className="mi-reveal" data-open={open} inert={!open}>
      <div>{children}</div>
    </div>
  );
}
