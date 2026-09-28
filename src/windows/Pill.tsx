// The dictation pill: floats bottom center above every app, never takes
// focus, and only accepts clicks while it shows buttons.

import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { Icon } from "../components/Icons";
import { api, type PillView } from "../lib/ipc";

const BARS = 22;

/** Microphone RMS to a bar height in px: -60 dB is silence, -15 dB is loud. */
function barHeight(level: number): number {
  const db = 20 * Math.log10(Math.max(level, 1e-5));
  const norm = Math.min(1, Math.max(0, (db + 60) / 45));
  return 4 + Math.round(norm * 24);
}

/** The tail of the live text: older words muted, the newest two bright. */
function tail(text: string): { older: string; newest: string } {
  const trimmed = text.trim();
  if (!trimmed) return { older: "", newest: "" };
  if (/\s/.test(trimmed)) {
    const words = trimmed.split(/\s+/).slice(-9);
    return { older: words.slice(0, -2).join(" "), newest: words.slice(-2).join(" ") };
  }
  // Chinese and Japanese have no spaces: show the last characters.
  const chars = Array.from(trimmed).slice(-22);
  return { older: chars.slice(0, -4).join(""), newest: chars.slice(-4).join("") };
}

function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function Chip({
  children,
  onClick,
  label,
  primary = false,
}: {
  children: React.ReactNode;
  onClick: () => void;
  label?: string;
  primary?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      onClick={onClick}
      className={
        "inline-flex h-8 items-center gap-1.5 rounded-full border-0 px-3 text-[13px] font-medium " +
        (primary ? "bg-white text-pill hover:bg-white/90" : "bg-white/[0.14] text-white hover:bg-white/[0.24]")
      }
    >
      {children}
    </button>
  );
}

function Shell({ children, tight = false }: { children: React.ReactNode; tight?: boolean }) {
  return (
    <div
      className={
        "pill-in flex h-[52px] items-center gap-3 whitespace-nowrap rounded-full bg-pill pl-[18px] text-sm text-white shadow-[0_12px_32px_rgba(0,0,0,.28)] " +
        (tight ? "pr-2" : "pr-[18px]")
      }
    >
      {children}
    </div>
  );
}

function Listening({ view, levels, text }: { view: Extract<PillView, { kind: "listening" }>; levels: number[]; text: string }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(timer);
  }, []);
  const { older, newest } = tail(text);
  return (
    <Shell>
      <span
        aria-hidden="true"
        className="size-[9px] rounded-full bg-live"
        style={{ boxShadow: "0 0 0 4px rgba(255,107,61,0.25)" }}
      />
      <div className="flex h-7 items-center gap-[3px]" aria-hidden="true">
        {levels.map((level, i) => (
          <div
            key={i}
            className="w-[3px] rounded-[2px] bg-wave transition-[height] duration-100 ease-out"
            style={{ height: barHeight(level) }}
          />
        ))}
      </div>
      <div className="flex w-[320px] justify-end overflow-hidden text-[15px]">
        {newest ? (
          <>
            <span className="text-white/60">{older}</span>
            <span>&nbsp;{newest}</span>
          </>
        ) : (
          <span className="text-white/60">{view.live ? "Listening…" : "Listening · words appear after each pause"}</span>
        )}
      </div>
      <span className="font-mono text-xs text-white/60">{clock(now - view.startedAt)}</span>
      {view.context && (
        <span className="inline-flex h-8 items-center rounded-full bg-white/[0.14] px-3 text-[13px] font-medium">
          {view.context}
        </span>
      )}
    </Shell>
  );
}

function Dots() {
  const [tick, setTick] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => setTick((t) => t + 1), 260);
    return () => clearInterval(timer);
  }, []);
  return (
    <span className="flex gap-1" aria-hidden="true">
      {[0, 1, 2].map((i) => (
        <span key={i} className="size-[5px] rounded-full bg-white" style={{ opacity: (tick + 3 - i) % 3 === 0 ? 1 : 0.35 }} />
      ))}
    </span>
  );
}

export function Pill() {
  const [view, setView] = useState<PillView>({ kind: "idle" });
  const [levels, setLevels] = useState<number[]>(() => Array(BARS).fill(0));
  const [partial, setPartial] = useState("");
  const token = useRef<number | null>(null);
  const box = useRef<HTMLDivElement>(null);

  // Fit the window to the pill: while it takes clicks, the transparent
  // space around it must not block the apps underneath.
  useEffect(() => {
    if (!box.current) return;
    const observer = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      api.fitPill(Math.ceil(width), Math.ceil(height)).catch(console.error);
    });
    observer.observe(box.current);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    api.state().then((s) => setView(s.pill), console.error);
    const stops = [
      listen<PillView>("pill-state", ({ payload }) => {
        if (payload.kind === "listening" && payload.token !== token.current) {
          token.current = payload.token;
          setPartial("");
          setLevels(Array(BARS).fill(0));
        }
        setView(payload);
      }),
      listen<number>("pill-level", ({ payload }) => setLevels((old) => [...old.slice(1), payload])),
      listen<[number, string]>("pill-partial", ({ payload: [from, text] }) => {
        if (from === token.current) setPartial(text);
      }),
    ];
    return () => stops.forEach((stop) => stop.then((f) => f()));
  }, []);

  let body: React.ReactNode;
  switch (view.kind) {
    case "idle":
      // Nothing on screen between dictations. The window stays open (showing
      // it again could take focus) and lets clicks through while idle.
      body = null;
      break;
    case "listening":
      body = <Listening view={view} levels={levels} text={partial} />;
      break;
    case "transcribing":
      body = (
        <Shell>
          <Icon name="sparkle" />
          <span>{view.label}</span>
          <Dots />
        </Shell>
      );
      break;
    case "polishing":
      body = (
        <Shell tight>
          <Icon name="sparkle" />
          <span>Polishing</span>
          <Dots />
          <Chip label="Skip polishing and insert the text as recognized" onClick={() => api.pill("skipPolish")}>
            Skip
          </Chip>
        </Shell>
      );
      break;
    case "inserted":
      body = (
        <Shell tight>
          <Icon name="check" className="text-ok" />
          <span>{view.label}</span>
          <Chip onClick={() => api.pill("undo")}>
            <Icon name="undo" size={14} />
            Undo
          </Chip>
          {view.canRaw && <Chip onClick={() => api.pill("useRaw")}>Use raw</Chip>}
        </Shell>
      );
      break;
    case "copied":
      body = (
        <Shell>
          <Icon name="copy" />
          <span>{view.label}</span>
          <span className="font-mono text-xs text-white/60">{view.hint}</span>
        </Shell>
      );
      break;
    case "failed":
      body = (
        <Shell tight>
          <Icon name="warning" className="text-warn" />
          <span title={view.detail}>{view.message}</span>
          {view.retryable && <Chip onClick={() => api.pill("retry")}>Retry</Chip>}
          {view.alternative && <Chip onClick={() => api.pill("switchEngine")}>{view.alternative}</Chip>}
          <Chip label="Discard" onClick={() => api.pill("dismiss")}>
            <Icon name="close" size={14} />
          </Chip>
        </Shell>
      );
      break;
    case "hint":
      body = (
        <div className="pill-in flex h-[52px] items-center rounded-full bg-pill/[0.88] px-[18px] text-sm text-white shadow-[0_12px_32px_rgba(0,0,0,.28)]">
          {view.text}
        </div>
      );
      break;
  }

  return (
    <div role="status" aria-live="polite" className="flex h-full items-end justify-center pb-4">
      <div ref={box} className="flex shrink-0">
        {body}
      </div>
    </div>
  );
}
