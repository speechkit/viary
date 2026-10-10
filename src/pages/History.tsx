// History (History artboard): what you said, which engine heard it, the
// raw and punctuated text, and the recording while it is kept.

import { convertFileSrc } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState } from "react";
import { Icon } from "../components/Icons";
import { api, errorText, useHistory, type HistoryItem, type Snapshot } from "../lib/ipc";

const base = "inline-flex h-9 shrink-0 items-center whitespace-nowrap gap-2 rounded-[9px] border px-3.5 text-sm font-medium disabled:opacity-50";
const btn = `${base} border-edge bg-white text-ink`;
const btnPrimary = `${base} border-ink bg-ink text-white`;

function time(ms: number) {
  return new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
}

function duration(ms: number) {
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function dayLabel(ms: number): string {
  const day = new Date(ms).toDateString();
  const today = new Date();
  if (day === today.toDateString()) return "Today";
  const yesterday = new Date(today.getTime() - 86_400_000);
  if (day === yesterday.toDateString()) return "Yesterday";
  return new Date(ms).toLocaleDateString(undefined, { weekday: "long", month: "short", day: "numeric" });
}

/** Words, or single characters for CJK text, keeping the spaces. */
function tokens(text: string): string[] {
  return text.match(/[぀-ヿ㐀-鿿가-힯]|[^\s぀-ヿ㐀-鿿가-힯]+|\s+/g) ?? [];
}

type Piece = { kind: "same" | "del" | "ins"; text: string };

/** A word diff of `a` into `b` (longest common subsequence). */
function diff(a: string, b: string): Piece[] {
  const x = tokens(a);
  const y = tokens(b);
  const n = x.length;
  const m = y.length;
  const lcs = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--)
      lcs[i][j] = x[i] === y[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
  const out: Piece[] = [];
  const push = (kind: Piece["kind"], text: string) => {
    const last = out[out.length - 1];
    if (last?.kind === kind) last.text += text;
    else out.push({ kind, text });
  };
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (x[i] === y[j]) push("same", x[i++]), j++;
    else if (lcs[i + 1][j] >= lcs[i][j + 1]) push("del", x[i++]);
    else push("ins", y[j++]);
  }
  while (i < n) push("del", x[i++]);
  while (j < m) push("ins", y[j++]);
  return out;
}

/** Peaks of the recording, decoded in the web view. */
function useWaveform(src: string | null, bars: number): number[] | null {
  const [peaks, setPeaks] = useState<number[] | null>(null);
  useEffect(() => {
    setPeaks(null);
    if (!src) return;
    let cancelled = false;
    (async () => {
      const bytes = await (await fetch(src)).arrayBuffer();
      const context = new AudioContext();
      const audio = await context.decodeAudioData(bytes);
      context.close();
      const data = audio.getChannelData(0);
      const step = Math.max(1, Math.floor(data.length / bars));
      const values: number[] = [];
      for (let b = 0; b < bars; b++) {
        let peak = 0;
        for (let k = b * step; k < Math.min(data.length, (b + 1) * step); k++) peak = Math.max(peak, Math.abs(data[k]));
        values.push(peak);
      }
      const top = Math.max(...values, 1e-3);
      if (!cancelled) setPeaks(values.map((v) => v / top));
    })().catch(console.error);
    return () => {
      cancelled = true;
    };
  }, [src, bars]);
  return peaks;
}

function Player({ item, keepDays }: { item: HistoryItem; keepDays: number }) {
  const src = item.recordingPath ? convertFileSrc(item.recordingPath) : null;
  const peaks = useWaveform(src, 72);
  const audio = useRef<HTMLAudioElement>(null);
  const [playing, setPlaying] = useState(false);
  const [progress, setProgress] = useState(0);
  if (!src) {
    return (
      <div className="rounded-xl border border-line bg-white px-4 py-3 text-xs text-faint">
        {keepDays === 0 ? "Recordings are not kept. Change this under Voice engine." : "The recording is no longer kept."}
      </div>
    );
  }
  return (
    <div className="flex items-center gap-3 rounded-xl border border-line bg-white px-4 py-3">
      <audio
        ref={audio}
        src={src}
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={() => {
          setPlaying(false);
          setProgress(0);
        }}
        onTimeUpdate={(e) => setProgress(e.currentTarget.currentTime / (e.currentTarget.duration || 1))}
      />
      <button
        type="button"
        className={`${btn} w-9 justify-center px-0`}
        aria-label={playing ? "Pause recording" : "Play recording"}
        onClick={() => (playing ? audio.current?.pause() : audio.current?.play())}
      >
        <Icon name={playing ? "pause" : "play"} />
      </button>
      <div className="flex h-7 grow items-center gap-0.5" aria-hidden="true">
        {(peaks ?? new Array(72).fill(0.1)).map((p, i) => (
          <div
            key={i}
            className="grow rounded-[1px]"
            style={{ height: 4 + Math.round(p * 22), background: i / 72 < progress ? "#1C1B18" : "#CFC8B8" }}
          />
        ))}
      </div>
      <span className="text-xs text-faint">Recording kept {keepDays} days</span>
    </div>
  );
}

function Detail({ item, s }: { item: HistoryItem; s: Snapshot }) {
  const hasChanges = !!item.punctuated && item.punctuated !== item.raw;
  const [view, setView] = useState<"final" | "raw" | "changes">("final");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    setView("final");
    setError("");
  }, [item.id]);
  const pieces = useMemo(() => (hasChanges ? diff(item.raw, item.punctuated!) : []), [item, hasChanges]);
  const removed = pieces.filter((p) => p.kind === "del" && p.text.trim()).length;
  const added = pieces.filter((p) => p.kind === "ins" && p.text.trim()).length;
  const seconds = item.durationMs / 1000;
  const wpm = seconds > 2 ? Math.round(item.words / (seconds / 60)) : null;
  const engine = s.engine.active;

  const retranscribe = async () => {
    setBusy(true);
    setError("");
    try {
      await api.retranscribe(item.id);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  // After "Use raw" the inserted text is the raw one, and the first tab
  // offers the punctuated version instead.
  const usedRaw = hasChanges && item.text === item.raw;
  const shown = view === "raw" ? item.raw : usedRaw ? item.punctuated! : item.text;
  return (
    <main className="flex w-0 grow flex-col gap-5 px-10 pt-9 pb-8">
      <div className="flex flex-wrap items-center gap-2">
        <span className="inline-flex h-[26px] items-center rounded-full bg-sand px-2.5 text-xs text-muted">{item.app || "Unknown app"}</span>
        <span className="inline-flex h-[26px] items-center rounded-full bg-sand px-2.5 text-xs text-muted">
          {time(item.createdAt)} · {duration(item.durationMs)}
        </span>
        {item.text && (
          <span className="inline-flex h-[26px] items-center rounded-full bg-sand px-2.5 text-xs text-muted">
            {item.words} words{wpm ? ` · ${wpm} wpm` : ""}
          </span>
        )}
        <span
          title={item.engine.name}
          className={
            "inline-flex h-[26px] items-center rounded-full px-2.5 text-xs " +
            (item.engine.onDevice ? "bg-teal-wash text-teal-ink" : "bg-blue-wash text-blue")
          }
        >
          {item.engine.kind || item.engine.name} · {item.engine.onDevice ? "on-device" : `cloud · ${item.engine.name}`}
        </span>
        {item.status === "undone" && <span className="text-xs text-faint">Undone</span>}
        {item.status === "copied" && <span className="text-xs text-faint">Copied to clipboard</span>}
      </div>

      {item.status === "failed" ? (
        <div className="flex flex-col gap-2 rounded-[14px] border border-line bg-white px-8 py-7">
          <span className="flex items-center gap-2 text-[15px] font-semibold text-rust">
            <Icon name="warning" /> This dictation failed
          </span>
          <span className="selectable text-sm text-body">{item.error}</span>
          <span className="text-[13px] text-muted">
            {item.recordingPath ? "The recording is kept: re-transcribe it with the current engine." : "The recording was not kept."}
          </span>
        </div>
      ) : (
        <>
          {hasChanges && (
            <div className="flex items-center justify-between">
              <div role="tablist" aria-label="Transcript view" className="inline-flex gap-0.5 rounded-[9px] bg-sand p-[3px]">
                {(
                  [
                    ["final", usedRaw ? "Punctuated" : "Inserted"],
                    ["raw", usedRaw ? "Raw · inserted" : "Raw"],
                    ["changes", "Changes"],
                  ] as const
                ).map(([id, label]) => (
                  <button
                    key={id}
                    type="button"
                    role="tab"
                    aria-selected={view === id}
                    onClick={() => setView(id)}
                    className={
                      "h-[30px] rounded-[7px] px-3.5 text-[13px] font-medium " +
                      (view === id ? "bg-white text-ink shadow-[0_1px_2px_rgba(0,0,0,.12)]" : "text-muted")
                    }
                  >
                    {label}
                  </button>
                ))}
              </div>
              {view === "changes" && (
                <span className="flex gap-3.5 text-[13px] text-muted">
                  <span>
                    <del className="rounded-[3px] bg-rust-wash px-0.5 text-rust">removed</del> {removed}
                  </span>
                  <span>
                    <ins className="rounded-[3px] bg-[#DDF0EC] px-0.5 text-teal-ink no-underline">added</ins> {added}
                  </span>
                </span>
              )}
            </div>
          )}
          <article className="selectable min-h-[160px] grow rounded-[14px] border border-line bg-white px-8 py-7 text-[17px] leading-[1.75] whitespace-pre-wrap">
            {view === "changes"
              ? pieces.map((p, i) =>
                  p.kind === "same" ? (
                    <span key={i}>{p.text}</span>
                  ) : p.kind === "del" ? (
                    <del key={i} className="rounded-[3px] bg-rust-wash px-0.5 text-rust decoration-rust">
                      {p.text}
                    </del>
                  ) : (
                    <ins key={i} className="rounded-[3px] bg-[#DDF0EC] px-0.5 text-teal-ink no-underline">
                      {p.text}
                    </ins>
                  ),
                )
              : shown}
          </article>
        </>
      )}

      <Player item={item} keepDays={s.settings.keepRecordingsDays} />

      {error && (
        <div role="alert" className="rounded-[10px] bg-rust-wash px-3 py-2.5 text-[13px] text-rust">
          {error}
        </div>
      )}
      <div className="flex gap-2.5">
        {item.text && (
          <button
            type="button"
            className={btnPrimary}
            onClick={() => {
              api.copy(shown);
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            }}
          >
            <Icon name="copy" />
            {copied ? "Copied" : "Copy"}
          </button>
        )}
        <button
          type="button"
          className={btn}
          disabled={busy || !item.recordingPath || !engine}
          title={engine ? `Transcribe the recording again with ${engine.name}` : "Choose a voice engine first"}
          onClick={retranscribe}
        >
          <Icon name="refresh" />
          {busy ? "Transcribing…" : "Re-transcribe"}
        </button>
        <div className="grow" />
        <button type="button" className={`${base} border-edge bg-white text-rust`} onClick={() => api.deleteHistory(item.id)}>
          Delete
        </button>
      </div>
    </main>
  );
}

export function HistoryPage({ s, focus }: { s: Snapshot; focus: string | null }) {
  const [items] = useHistory();
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<string | null>(focus);
  useEffect(() => {
    if (focus) setSelected(focus);
  }, [focus]);

  const list = (items ?? []).filter((h) => !query || `${h.text} ${h.raw} ${h.app}`.toLowerCase().includes(query.toLowerCase()));
  const current = list.find((h) => h.id === selected) ?? list[0];
  let lastDay = "";

  return (
    <div className="flex h-full">
      <section aria-label="Dictations" className="flex w-[380px] shrink-0 flex-col border-r border-rule">
        <div className="flex flex-col gap-3.5 px-6 pt-9 pb-4">
          <h1 className="m-0 font-serif text-[34px] font-normal">History</h1>
          <label className="flex h-[38px] items-center gap-2 rounded-[10px] border border-rule bg-white px-3 text-faint">
            <Icon name="search" />
            <input
              type="search"
              placeholder="Search what you said"
              aria-label="Search history"
              className="grow border-0 bg-transparent text-sm text-ink outline-none"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </label>
        </div>
        <div className="scroll grow pb-4">
          {list.length === 0 && <p className="px-6 text-sm text-muted">{query ? "Nothing matches." : "No dictations yet."}</p>}
          {list.map((item) => {
            const day = dayLabel(item.createdAt);
            const header = day !== lastDay;
            lastDay = day;
            const on = item.id === current?.id;
            return (
              <div key={item.id}>
                {header && (
                  <span className="block px-6 pt-2 pb-2 text-xs font-semibold tracking-[0.04em] text-faint uppercase">{day}</span>
                )}
                <button
                  type="button"
                  onClick={() => setSelected(item.id)}
                  aria-current={on}
                  className={
                    "mx-3 flex w-[calc(100%-24px)] flex-col gap-1 rounded-[10px] p-3 text-left " + (on ? "bg-white" : "hover:bg-white/50")
                  }
                >
                  <div className="flex w-full items-baseline justify-between gap-3">
                    <span className="truncate text-[13px] font-semibold">{item.app || "Unknown app"}</span>
                    <span className="font-mono text-xs text-faint">{time(item.createdAt)}</span>
                  </div>
                  <span className="line-clamp-2 text-[13px] leading-[1.45] text-body">
                    {item.status === "failed" ? <span className="text-rust">Failed · {item.error}</span> : item.text}
                  </span>
                </button>
              </div>
            );
          })}
        </div>
      </section>
      {current ? (
        <Detail key={current.id} item={current} s={s} />
      ) : (
        <main className="flex grow items-center justify-center text-sm text-muted">
          {`Hold ${s.hotkeyName} and speak to make your first dictation.`}
        </main>
      )}
    </div>
  );
}
