// Voice Notes (VoiceNotesEmpty, VoiceNoteRecording, VoiceNotes artboards;
// NotesPrototype is the behavior spec). Notes are kept apart from
// dictation History.
//
// Speakers and word timing show only when the note has them: speechkit 0.5
// reports neither, so real notes have passages with their own times, and
// the player highlights whole passages.

import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { useEffect, useMemo, useRef, useState } from "react";
import { Icon, Kbd, RecordIcon } from "../components/Icons";
import { Menu } from "../components/Menu";
import { PlayerControls } from "../components/Player";
import { MetaChip, SidebarEmpty, ToolSidebar, useToolSearch } from "../components/ToolSidebar";
import {
  api,
  clock,
  errorText,
  useMicrophoneName,
  useNotes,
  useRecorder,
  type LiveLine,
  type Note,
  type NoteFormat,
  type Passage,
  type RecorderState,
  type Snapshot,
} from "../lib/ipc";
import { usePlayback, type Playback } from "../lib/playback";
import { speakerColor } from "../lib/speakers";
import { currentIndex, daysAgo, spaced } from "../lib/transcript";
import type { Intent } from "../windows/MainWindow";


const lbl = "text-[11px] font-semibold tracking-[0.06em] text-faint uppercase";
const pillBtn = "inline-flex h-11 shrink-0 items-center whitespace-nowrap justify-center gap-2 rounded-full border px-[18px] text-sm font-medium";
const btn = "inline-flex h-9 items-center whitespace-nowrap justify-center gap-2 rounded-[9px] border border-edge bg-white px-3.5 text-[13px] font-medium hover:bg-card";

/** Where the summary goes: on this Mac only with a local polish server. */
function summaryOnDevice(s: Snapshot) {
  return !s.settings.polish.enabled || s.settings.polish.provider === "local";
}

function PrivacyChip({ onDevice, kind, long }: { onDevice: boolean; kind?: string; long?: boolean }) {
  if (!onDevice) return <MetaChip tone="cloud">Cloud{kind ? ` · ${kind}` : ""}</MetaChip>;
  return <MetaChip tone="device">{long ? "On-device · nothing leaves this Mac" : "On-device"}</MetaChip>;
}

// ---------------------------------------------------------------------------
// Sidebar

function day(ms: number): string {
  const date = new Date(ms);
  const days = daysAgo(ms);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return "Earlier this week";
  return date.toLocaleDateString(undefined, { month: "long", year: days > 300 ? "numeric" : undefined });
}

function when(ms: number, long = false): string {
  const date = new Date(ms);
  const time = date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const label = day(ms);
  if (label === "Today") return long ? `Today ${time}` : time;
  if (label === "Yesterday") return long ? `Yesterday ${time}` : "Yesterday";
  const weekday = date.toLocaleDateString(undefined, { weekday: "short" });
  if (label === "Earlier this week") return long ? `${weekday} ${time}` : weekday;
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" }) + (long ? ` ${time}` : "");
}

function words(note: Note): number {
  return note.passages.reduce((n, p) => n + (p.text.match(/[\p{L}\p{N}]+/gu)?.length ?? 0), 0);
}

/** "10:30 · 2 speakers", or the word count when speakers are not known. */
function listMeta(note: Note): string {
  const n = note.speakers.length;
  const who = n > 1 ? `${n} speakers` : n === 1 ? "just you" : `${words(note)} words`;
  return `${when(note.createdAt)} · ${who}`;
}

function NoteList({
  notes,
  query,
  selected,
  recorder,
  onPick,
}: {
  notes: Note[];
  query: string;
  selected: string | null;
  recorder: RecorderState | null;
  onPick: (id: string) => void;
}) {
  const q = query.trim().toLowerCase();
  const busy = recorder && (recorder.phase === "recording" || recorder.phase === "paused");
  const items = notes
    .map((note) => {
      if (!q) return { note, hit: null as Passage | null };
      const hit = note.passages.find((p) => p.text.toLowerCase().includes(q)) ?? null;
      return note.title.toLowerCase().includes(q) || hit ? { note, hit } : null;
    })
    .filter((x) => x !== null);

  const groups: { label: string; items: typeof items }[] = [];
  for (const item of items) {
    const label = q ? `${items.length} ${items.length === 1 ? "match" : "matches"}` : day(item.note.createdAt);
    const last = groups[groups.length - 1];
    if (last?.label === label) last.items.push(item);
    else groups.push({ label, items: [item] });
  }

  if (!busy && notes.length === 0) {
    return <SidebarEmpty icon={<Icon name="notes" size={28} strokeWidth={1.5} />} title="No notes yet" text="Your recordings will show up here, newest first." />;
  }
  // The note being recorded leads Today.
  if (busy && !q && groups[0]?.label !== "Today") groups.unshift({ label: "Today", items: [] });
  return (
    <div className="scroll flex grow flex-col pb-4">
      {groups.map((group, g) => (
        <div key={group.label} className="flex flex-col">
          <span className={lbl + " px-5 pt-2.5 pb-1.5"}>{group.label}</span>
          {busy && !q && g === 0 && <RecordingItem recorder={recorder} />}
          {group.items.map(({ note, hit }) => {
            const current = note.id === selected;
            return (
              <button
                key={note.id}
                type="button"
                onClick={() => onPick(note.id)}
                aria-current={current ? "page" : undefined}
                className={
                  "mx-2.5 mb-0.5 flex flex-col gap-[3px] rounded-[10px] px-3 py-[11px] text-left " +
                  (current ? "bg-white" : "hover:bg-white/60")
                }
              >
                <span className="flex w-full justify-between gap-2">
                  <span className="truncate text-sm font-semibold">{note.title}</span>
                  <span className="shrink-0 font-mono text-xs text-faint">{clock(note.durationMs)}</span>
                </span>
                <span className="w-full truncate text-xs text-muted">{hit ? `${clock(hit.startMs)}  ${hit.text}` : listMeta(note)}</span>
              </button>
            );
          })}
        </div>
      ))}
      {q && items.length === 0 && <span className="px-5 py-4 text-[13px] text-faint">No notes mention “{query.trim()}”.</span>}
    </div>
  );
}

/** Recorded time now: before the running part, plus the running part. */
function useElapsed(recorder: RecorderState | null): number {
  const [now, setNow] = useState(Date.now());
  const running = recorder?.runningSince ?? null;
  useEffect(() => {
    if (running === null) return;
    const timer = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(timer);
  }, [running]);
  if (!recorder) return 0;
  return recorder.elapsedMs + (running === null ? 0 : Math.max(0, now - running));
}

function RecordingItem({ recorder }: { recorder: RecorderState }) {
  const elapsed = useElapsed(recorder);
  const paused = recorder.phase === "paused";
  return (
    <div className="mx-2.5 mb-0.5 flex flex-col gap-[3px] rounded-[10px] bg-white px-3 py-[11px] shadow-[inset_0_0_0_1.5px_var(--color-record)]">
      <div className="flex items-center justify-between gap-2">
        <span className="flex min-w-0 items-center gap-2 text-sm font-semibold">
          <span aria-hidden="true" className={"size-2 shrink-0 rounded-full " + (paused ? "bg-stone" : "bg-record")} />
          <span className="truncate">{recorder.title}</span>
        </span>
        <span className="font-mono text-xs text-rust">{clock(elapsed)}</span>
      </div>
      <span className="text-xs text-rust">{paused ? "Paused" : "Recording…"}</span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Empty

function Empty({ s, onRecord }: { s: Snapshot; onRecord: () => void }) {
  const microphone = useMicrophoneName(s);
  const active = s.engine.active;
  return (
    <main className="flex grow flex-col items-center justify-center gap-7 px-16 py-10 text-center">
      <button
        type="button"
        onClick={onRecord}
        aria-label="Start recording (⌥⌘N)"
        title="Start recording (⌥⌘N)"
        className="flex size-[132px] items-center justify-center rounded-full bg-white shadow-[0_0_0_10px_rgba(224,69,43,0.10),0_14px_34px_rgba(28,27,24,0.16),inset_0_0_0_2px_#E7E1D5]"
      >
        <span aria-hidden="true" className="size-16 rounded-full bg-record" />
      </button>
      <div className="flex flex-col items-center gap-2.5">
        <h2 className="m-0 font-serif text-4xl font-normal">Record your first note</h2>
        <p className="m-0 max-w-[480px] text-[15px] leading-[1.6] text-muted">
          Talk through an idea, or record a conversation. Viary writes it down as you speak
          {afterStop(s) ? `, and adds ${afterStop(s)} when you stop.` : "."}
        </p>
      </div>
      <div className="flex items-center gap-2 text-[13px] text-muted">
        <span>Press</span>
        <Kbd>⌥⌘N</Kbd>
        <span>from any app, or choose New voice note in the menu bar.</span>
      </div>
      <div className="flex gap-2">
        <MetaChip>
          <Icon name="mic" size={13} />
          {microphone}
        </MetaChip>
        <PrivacyChip onDevice={!active || (active.onDevice && summaryOnDevice(s))} kind={active?.kind} long />
      </div>
    </main>
  );
}

/** What finishing a note adds, in words: "speakers, a summary and action items". */
function afterStop(s: Snapshot, timing = false): string {
  const parts: string[] = [];
  if (s.speechCaps.speakers) parts.push("speakers");
  if (s.settings.polish.enabled) parts.push("a summary", "action items");
  if (timing && s.speechCaps.wordTimings) parts.push("exact word timing");
  if (parts.length < 2) return parts.join("");
  return parts.slice(0, -1).join(", ") + " and " + parts[parts.length - 1];
}

// ---------------------------------------------------------------------------
// Recording

/** One bar per quarter second; the window shows the last 30 s. Bars and
 *  heights match the player's waveform, so the two read as one. */
const BARS = 120;
const BAR_MS = 250;

function barHeight(level: number): number {
  const db = 20 * Math.log10(Math.max(level, 1e-5));
  const norm = Math.min(1, Math.max(0, (db + 60) / 45));
  return 3 + Math.round(norm * 27);
}

/** Microphone levels, folded to the loudest per quarter second. */
function useLevels(running: boolean): number[] {
  const [bars, setBars] = useState<number[]>(() => Array(BARS).fill(0));
  const peak = useRef(0);
  useEffect(() => {
    const stop = listen<number>("note-level", ({ payload }) => {
      peak.current = Math.max(peak.current, payload);
    });
    return () => {
      stop.then((f) => f());
    };
  }, []);
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(() => {
      const level = peak.current;
      peak.current = 0;
      setBars((old) => [...old.slice(1), level]);
    }, BAR_MS);
    return () => clearInterval(timer);
  }, [running]);
  return bars;
}

interface Live {
  lines: LiveLine[];
  partial: string;
  /** Where the speech behind `partial` started, when the engine reports it. */
  partialStartMs: number | null;
}

function useLive(id: string | null): Live {
  const [live, setLive] = useState<Live>({ lines: [], partial: "", partialStartMs: null });
  useEffect(() => {
    setLive({ lines: [], partial: "", partialStartMs: null });
    // An update that arrives first is newer than what was asked for.
    let updated = false;
    const stop = listen<Live & { id: string }>("note-live", ({ payload }) => {
      if (payload.id !== id) return;
      updated = true;
      setLive(payload);
    });
    // Opened mid-recording: what was said so far, even during a silence.
    api.noteLive().then((now) => {
      if (now?.id === id && !updated) setLive(now);
    }, console.error);
    return () => {
      stop.then((f) => f());
    };
  }, [id]);
  return live;
}

function Recording({ s, recorder }: { s: Snapshot; recorder: RecorderState }) {
  const microphone = useMicrophoneName(s);
  const paused = recorder.phase === "paused";
  const elapsed = useElapsed(recorder);
  const bars = useLevels(!paused);
  const live = useLive(recorder.id);
  const [title, setTitle] = useState(recorder.title);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const transcript = useRef<HTMLElement>(null);
  const caption = afterStop(s, true);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && e.key.toLowerCase() === "m" && !paused) {
        e.preventDefault();
        api.noteMark().catch(console.error);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [paused]);

  useEffect(() => {
    transcript.current?.scrollTo({ top: transcript.current.scrollHeight });
  }, [live]);

  const windowMs = BARS * BAR_MS;
  const marks = recorder.marks
    .map((at) => ({ at, left: 100 - ((elapsed - at) / windowMs) * 100 }))
    .filter((m) => m.left > 0);

  // Recognized lines, then the words still being recognized.
  const lines: { startMs: number | null; text: string; partial: string }[] = live.lines.map((l) => ({ ...l, partial: "" }));
  if (live.partial) lines.push({ startMs: live.partialStartMs, text: "", partial: live.partial });

  return (
    <main className="flex w-0 grow flex-col gap-[22px] px-12 pt-11 pb-7">
      <div className="flex items-center gap-3">
        <span
          className={
            "inline-flex h-[30px] shrink-0 items-center gap-2 rounded-full px-3 text-[13px] font-semibold " +
            (paused ? "bg-sand text-muted" : "bg-rust-wash text-rust")
          }
        >
          <span aria-hidden="true" className={"size-[9px] rounded-full " + (paused ? "bg-muted" : "bg-record")} />
          {paused ? "Paused" : "Recording"}
        </span>
        <input
          aria-label="Note title"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onBlur={() => recorder.id && title.trim() && api.noteRename(recorder.id, title.trim())}
          onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          className="min-w-0 grow border-0 bg-transparent font-serif text-[30px] text-ink outline-0"
        />
        <MetaChip>
          <Icon name="mic" size={13} />
          {microphone}
        </MetaChip>
        <PrivacyChip onDevice={s.engine.active?.onDevice ?? true} kind={s.engine.active?.kind} />
      </div>

      <div aria-hidden="true" className="shrink-0 rounded-[14px] border border-line bg-white px-4 py-3">
        <div className="relative flex h-[34px] items-center gap-0.5">
          {bars.map((level, i) => (
            <div key={i} className="grow rounded-[1px]" style={{ height: barHeight(level), background: i > BARS - 10 ? "#FF8A5C" : "#1C1B18" }} />
          ))}
          {marks.map((m) => (
            <span
              key={m.at}
              title={`Marked at ${clock(m.at)}`}
              className="absolute -top-2 -ml-1 size-[9px] rotate-45 rounded-[2px] bg-[#D99A1E]"
              style={{ left: `${m.left}%` }}
            />
          ))}
        </div>
      </div>

      <article
        ref={transcript}
        aria-label="Live transcript"
        aria-live="polite"
        className="flex min-h-0 grow flex-col gap-3.5 overflow-hidden text-[17px] leading-[1.65]"
      >
        <div className="grow" />
        {lines.length === 0 && (
          <p className="m-0 text-[#8A847A]">
            {recorder.waitingForEngine
              ? "Recording. The voice engine is still loading; your words appear once it's ready."
              : "Start talking. Words appear here as Viary hears them."}
          </p>
        )}
        {lines.map((l, i) => (
          <div key={i} className="flex gap-3.5">
            <span className="w-12 shrink-0 pt-1 font-mono text-xs text-faint">{l.startMs === null ? "" : clock(l.startMs)}</span>
            <p className="selectable m-0">
              {l.text}
              {l.partial && <span className="text-[#8A847A]">{l.partial}</span>}
              {i === lines.length - 1 && !paused && (
                <span aria-hidden="true" className="ml-0.5 inline-block h-[18px] w-0.5 bg-record align-[-3px]" />
              )}
            </p>
          </div>
        ))}
      </article>

      <div className="flex items-center gap-3">
        <span className="w-[110px] font-mono text-[28px] font-medium">{clock(elapsed)}</span>
        <button
          type="button"
          disabled={recorder.waitingForEngine}
          title={recorder.waitingForEngine ? "Available once the voice engine has loaded" : undefined}
          onClick={() => (paused ? api.noteResume() : api.notePause()).catch(console.error)}
          className={pillBtn + " border-edge bg-white disabled:opacity-50"}
        >
          <Icon name={paused ? "play" : "pause"} />
          {paused ? "Resume" : "Pause"}
        </button>
        <button type="button" disabled={paused} onClick={() => api.noteMark().catch(console.error)} className={pillBtn + " border-edge bg-white disabled:opacity-50"}>
          <Icon name="flag" />
          Mark moment
          <span className="inline-flex h-5 items-center rounded-[5px] bg-sand px-1.5 font-mono text-[11px] text-muted">⌘M</span>
        </button>
        <div className="grow" />
        {caption && <span className="max-w-[260px] text-right text-[13px] leading-[1.45] text-muted">{capitalize(caption)} are added when you stop.</span>}
        {confirmDiscard ? (
          <button type="button" onClick={() => api.noteDiscard().catch(console.error)} onBlur={() => setConfirmDiscard(false)} autoFocus className={pillBtn + " border-rust bg-white text-rust"}>
            Discard recording?
          </button>
        ) : (
          <button type="button" onClick={() => setConfirmDiscard(true)} className={pillBtn + " border-transparent bg-transparent text-muted"}>
            Discard
          </button>
        )}
        <button
          type="button"
          disabled={recorder.waitingForEngine}
          title={recorder.waitingForEngine ? "Available once the voice engine has loaded" : undefined}
          onClick={() => api.noteStop().catch(console.error)}
          className={pillBtn + " border-record bg-record px-[22px] text-white disabled:opacity-50"}
        >
          <span aria-hidden="true" className="size-3 rounded-[3px] bg-white" />
          Stop &amp; save
        </button>
      </div>
    </main>
  );
}

const capitalize = (text: string) => text.charAt(0).toUpperCase() + text.slice(1);

// ---------------------------------------------------------------------------
// Finishing

function Finishing({ s, recorder }: { s: Snapshot; recorder: RecorderState }) {
  const local = (s.engine.active?.onDevice ?? true) && summaryOnDevice(s);
  return (
    <main className="flex grow items-center justify-center">
      <div role="status" className="flex w-[380px] flex-col gap-3.5 rounded-2xl border border-line bg-white px-7 py-6">
        <span className="font-serif text-[26px]">Finishing your note</span>
        {recorder.steps.map((step) => (
          <div key={step.label} className={"flex items-center gap-3 text-sm " + (step.state === "todo" ? "text-[#8A847A]" : "text-ink")}>
            <span
              className={
                "inline-flex size-5 shrink-0 items-center justify-center rounded-full text-white " +
                (step.state === "done" ? "bg-teal" : step.state === "now" ? "bg-blue" : "bg-rule")
              }
            >
              {step.state === "done" && <Icon name="check" size={12} strokeWidth={2.6} />}
            </span>
            {step.state === "now" ? `${step.label}…` : step.label}
          </div>
        ))}
        <span className="text-xs text-faint">{local ? "On-device. " : ""}You can keep dictating meanwhile.</span>
      </div>
    </main>
  );
}

// ---------------------------------------------------------------------------
// A note


function PassageText({ passage, pos, active }: { passage: Passage; pos: number; active: boolean }) {
  // Word-level highlight only with real word timing; otherwise the whole
  // passage is highlighted by its row.
  if (!active || !passage.words) return <>{passage.text}</>;
  const now = passage.words.findIndex((w) => pos >= w.startMs && pos < w.endMs);
  const spoken = now >= 0 ? now : passage.words.filter((w) => w.endMs <= pos).length - 1;
  return (
    <>
      {passage.words.map((w, i) => (
        <span key={i}>
          {i > 0 && spaced(passage.words![i - 1].text, w.text) ? " " : ""}
          <span className={i === now ? "rounded-[3px] bg-[#FFE3D3] shadow-[0_0_0_2px_#FFE3D3]" : i > spoken ? "text-[#8A847A]" : ""}>{w.text}</span>
        </span>
      ))}
    </>
  );
}

function Waveform({ note, playback, onUnmark }: { note: Note; playback: Playback; onUnmark: (ms: number) => void }) {
  const frac = note.durationMs ? Math.min(1, playback.pos / note.durationMs) : 0;
  const n = note.peaks.length;
  const bars = note.peaks.map((peak, i) => {
    const at = ((i + 0.5) / n) * note.durationMs;
    const passage = note.passages.find((p) => at >= p.startMs && at < p.endMs);
    const color = !passage ? "#CFC8B8" : note.speakers.length ? speakerColor(passage.speaker) : "#1C1B18";
    return { h: 3 + Math.round(peak * 27), color, played: i / n <= frac };
  });
  return (
    <div className="relative">
      <button
        type="button"
        title="Click to jump"
        aria-label="Jump to a point in the recording"
        disabled={!playback.path}
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          playback.seek(((e.clientX - r.left) / r.width) * note.durationMs, playback.playing);
        }}
        className="relative flex h-[34px] w-full items-center gap-0.5"
      >
        {bars.map((b, i) => (
          <div key={i} className="grow rounded-[1px]" style={{ height: b.h, background: b.color, opacity: b.played ? 1 : 0.35 }} />
        ))}
        <span aria-hidden="true" className="absolute -top-[3px] -bottom-[3px] w-0.5 rounded-[1px] bg-ink" style={{ left: `${frac * 100}%` }} />
      </button>
      {note.marks.map((m) => (
        <button
          key={m}
          type="button"
          title={`Marked at ${clock(m)} · click to jump, right-click to remove`}
          aria-label={`Mark at ${clock(m)}`}
          disabled={!playback.path}
          onClick={() => playback.seek(m)}
          onContextMenu={(e) => {
            e.preventDefault();
            onUnmark(m);
          }}
          className="absolute -top-2.5 -ml-[7px] flex size-3.5 items-center justify-center"
          style={{ left: `${(m / note.durationMs) * 100}%` }}
        >
          <span aria-hidden="true" className="size-[9px] rotate-45 rounded-[2px] bg-[#D99A1E]" />
        </button>
      ))}
    </div>
  );
}

/** The passage a mark belongs to: the one it falls in, or the one before it. */
const markedPassage = (passages: Passage[], mark: number) => Math.max(0, currentIndex(passages, mark));

function SummaryCard({ note, onSeek }: { note: Note; onSeek: (ms: number) => void }) {
  const [busy, setBusy] = useState(false);
  const write = () => {
    setBusy(true);
    api.noteSummarize(note.id).catch(console.error).finally(() => setBusy(false));
  };
  const empty = !note.summary && note.actions.length === 0;
  return (
    <div className="grid shrink-0 grid-cols-2 gap-5 rounded-[14px] border border-line bg-white px-5 py-4">
      <div className="flex flex-col gap-2">
        <span className={lbl}>Summary</span>
        {note.summary && <p className="selectable m-0 text-sm leading-[1.55]">{note.summary}</p>}
        {empty && (
          <span className="text-[13px] leading-[1.5] text-faint">
            {note.summaryError ? `Couldn't write a summary: ${note.summaryError}` : "No summary yet."}{" "}
            <button type="button" disabled={busy} onClick={write} className="font-semibold text-blue disabled:opacity-50">
              {busy ? "Writing…" : note.summaryError ? "Try again" : "Write one"}
            </button>
          </span>
        )}
      </div>
      <div className="flex flex-col gap-2">
        <span className={lbl}>Action items</span>
        {note.actions.map((a, i) => (
          <label key={i} className="flex items-start gap-2 text-sm">
            <input type="checkbox" checked={a.done} onChange={() => api.noteSetAction(note.id, i, !a.done).catch(console.error)} className="mt-[3px]" />
            <span className={a.done ? "text-[#8A847A] line-through" : ""}>
              {a.who && <b className="font-semibold">{a.who}</b>}
              {a.who && ": "}
              {a.text}{" "}
              {a.atMs !== null && (
                <button type="button" onClick={() => onSeek(a.atMs!)} className="font-mono text-xs text-faint hover:text-blue hover:underline">
                  {clock(a.atMs)}
                </button>
              )}
            </span>
          </label>
        ))}
        {!empty && note.actions.length === 0 && <span className="text-[13px] text-faint">None found.</span>}
      </div>
    </div>
  );
}

function NoteMenu({ note }: { note: Note }) {
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    await api.copy(await api.noteText(note.id, "markdown"));
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };
  const exportAs = async (format: NoteFormat) => {
    const ext = format === "markdown" ? "md" : "txt";
    const path = await save({
      defaultPath: `${note.title.replace(/[/:]/g, "-")}.${ext}`,
      filters: [{ name: format === "markdown" ? "Markdown" : "Text", extensions: [ext] }],
    });
    if (path) await api.noteExport(note.id, format, path);
  };

  return (
    <div className="flex shrink-0 gap-2">
      <button type="button" onClick={() => copy().catch(console.error)} className={btn}>
        <Icon name="copy" />
        {copied ? "Copied" : "Copy"}
      </button>
      <Menu
        label="Export"
        className={btn}
        items={[
          { label: "Markdown (.md)", onSelect: () => exportAs("markdown").catch(console.error) },
          { label: "Plain text (.txt)", onSelect: () => exportAs("text").catch(console.error) },
        ]}
      >
        Export ▾
      </Menu>
      <Menu
        label="More actions"
        className={btn + " w-9 px-0"}
        items={[{ label: "Delete note", danger: true, onSelect: () => api.noteDelete(note.id).catch(console.error) }]}
      >
        <Icon name="more" strokeWidth={3} />
      </Menu>
    </div>
  );
}

function Title({ note }: { note: Note }) {
  const [draft, setDraft] = useState<string | null>(null);
  if (draft === null) {
    return (
      <h2 onDoubleClick={() => setDraft(note.title)} title="Double-click to rename" className="m-0 min-w-0 font-serif text-[30px] leading-[1.15] font-normal">
        {note.title}
      </h2>
    );
  }
  const done = () => {
    if (draft.trim() && draft.trim() !== note.title) api.noteRename(note.id, draft.trim()).catch(console.error);
    setDraft(null);
  };
  return (
    <input
      aria-label="Note title"
      autoFocus
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={done}
      onKeyDown={(e) => {
        if (e.key === "Enter") done();
        if (e.key === "Escape") setDraft(null);
      }}
      className="min-w-0 grow border-0 bg-transparent p-0 font-serif text-[30px] leading-[1.15] outline-0"
    />
  );
}

function NoteView({ s, note }: { s: Snapshot; note: Note }) {
  const playback = usePlayback(note.audioPath, note.durationMs);
  const current = currentIndex(note.passages, playback.pos);
  const started = playback.playing || playback.pos > 0;
  const rows = useRef<(HTMLDivElement | null)[]>([]);

  // Keep the passage being played in view.
  useEffect(() => {
    if (playback.playing && current >= 0) rows.current[current]?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [current, playback.playing]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement;
      if (e.key === " " && !["INPUT", "TEXTAREA", "BUTTON"].includes(target.tagName)) {
        e.preventDefault();
        playback.toggle();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [playback]);

  const setMarks = (marks: number[]) => api.noteSetMarks(note.id, marks.map(Math.round)).catch(console.error);
  const marksOf = (i: number) => note.marks.filter((m) => markedPassage(note.passages, m) === i);
  const marked = (i: number) => marksOf(i).length > 0;
  // The flag on a passage marks its start, or clears its marks.
  const toggleMark = (i: number) => {
    const own = marksOf(i);
    setMarks(own.length ? note.marks.filter((m) => !own.includes(m)) : [...note.marks, note.passages[i].startMs]);
  };
  const unmark = (ms: number) => setMarks(note.marks.filter((m) => m !== ms));

  // ⌘M marks where playback is, as it marks the moment while recording.
  const markHere = useRef(() => {});
  markHere.current = () => {
    if (note.marks.some((m) => Math.abs(m - playback.pos) < 1000)) return;
    setMarks([...note.marks, playback.pos]);
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && e.key.toLowerCase() === "m") {
        e.preventDefault();
        markHere.current();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const engineOnDevice = note.engine.onDevice;

  return (
    <main className="flex w-0 grow flex-col">
      <div data-tauri-drag-region className="flex flex-col gap-3 px-11 pt-11">
        <div className="flex items-start justify-between gap-4">
          <Title key={note.id} note={note} />
          <NoteMenu note={note} />
        </div>
        <div className="flex flex-wrap gap-2">
          <MetaChip>{when(note.createdAt, true)}</MetaChip>
          <MetaChip>{clock(note.durationMs)}</MetaChip>
          {note.speakers.length > 0 && (
            <MetaChip>
              {note.speakers.map((sp, i) => (
                <span key={i} className="flex items-center gap-1.5">
                  <span className="ml-1 size-2 rounded-full first:ml-0" style={{ background: speakerColor(i) }} />
                  {sp.name}
                </span>
              ))}
            </MetaChip>
          )}
          <PrivacyChip onDevice={engineOnDevice} kind={note.engine.kind} />
        </div>
      </div>

      <div className="scroll flex min-h-0 grow flex-col gap-3.5 px-11 pt-[18px] pb-4">
        {s.settings.polish.enabled && <SummaryCard note={note} onSeek={(ms) => playback.seek(ms)} />}
        <article aria-label="Transcript" className="flex flex-col gap-1 text-[15px] leading-[1.6]">
          {note.passages.length === 0 && <p className="m-0 text-faint">Nothing was recognized in this recording.</p>}
          {note.passages.map((p, i) => {
            const isNow = i === current && started;
            return (
              <div
                key={i}
                ref={(el) => {
                  rows.current[i] = el;
                }}
                className="group -mx-3 flex gap-3 rounded-[10px] px-3 py-2 transition-colors duration-300"
                style={{ background: isNow ? "#FFFFFF" : "transparent" }}
              >
                <div className="flex w-12 shrink-0 flex-col items-start gap-1 pt-[3px]">
                  <button type="button" onClick={() => playback.seek(p.startMs)} aria-label={`Play from ${clock(p.startMs)}`} className="font-mono text-xs text-faint hover:text-blue hover:underline">
                    {clock(p.startMs)}
                  </button>
                  <button
                    type="button"
                    onClick={() => toggleMark(i)}
                    aria-pressed={marked(i)}
                    aria-label={marked(i) ? "Remove mark" : "Mark this passage"}
                    title={marked(i) ? "Remove mark" : "Mark this passage (⌘M marks where playback is)"}
                    className={
                      "rounded-[5px] p-0.5 opacity-0 group-hover:opacity-100 focus-visible:opacity-100 " +
                      (marked(i) ? "text-[#B7791F] hover:bg-[#FFF1D6]" : "text-faint hover:bg-sand hover:text-ink")
                    }
                  >
                    <Icon name="flag" size={13} />
                  </button>
                </div>
                <div className="min-w-0 grow">
                  {(note.speakers.length > 0 || marked(i)) && (
                    <div className="flex items-center gap-2 text-xs font-semibold" style={{ color: speakerColor(p.speaker) }}>
                      {p.speaker !== null && note.speakers[p.speaker]?.name}
                      {marked(i) && (
                        <span className="inline-flex h-5 items-center gap-1 rounded-full bg-[#FFF1D6] px-2 text-[11px] text-[#7A5200]">
                          <Icon name="flag" size={11} />
                          Marked
                        </span>
                      )}
                    </div>
                  )}
                  <span className={"selectable " + (started && i > current ? "text-[#8A847A]" : "")}>
                    <PassageText passage={p} pos={playback.pos} active={isNow} />
                  </span>
                </div>
              </div>
            );
          })}
        </article>
      </div>

      <div className="mx-6 mt-1 mb-5 flex flex-col gap-2 rounded-[14px] border border-line bg-white px-4 py-3">
        {note.peaks.length > 0 && <Waveform note={note} playback={playback} onUnmark={unmark} />}
        <PlayerControls playback={playback} durationMs={note.durationMs}>
          {!note.audioPath && (
            <span className="text-xs text-faint">
              {note.pendingAudio?.length ? "The recording couldn't be saved yet; Viary tries again when it next starts." : "The recording is no longer kept."}
            </span>
          )}
          {note.speakers.length > 0 && (
            <span className="flex items-center gap-1.5 text-xs text-faint">
              {note.speakers.map((sp, i) => (
                <span key={i} className="ml-1.5 flex items-center gap-1.5 first:ml-0">
                  <span className="h-1 w-2.5 rounded-sm" style={{ background: speakerColor(i) }} />
                  {sp.name}
                </span>
              ))}
            </span>
          )}
        </PlayerControls>
      </div>
    </main>
  );
}

// ---------------------------------------------------------------------------

function ErrorBar({ text }: { text: string }) {
  return (
    <div role="alert" className="absolute top-3 left-1/2 z-20 -translate-x-1/2 rounded-[10px] bg-rust-wash px-4 py-2 text-[13px] text-rust shadow-[0_4px_12px_rgba(28,27,24,0.1)]">
      {text}
    </div>
  );
}

export function VoiceNotesPage({ s, onBack, intent }: { s: Snapshot; onBack: () => void; intent: Intent }) {
  const search = useToolSearch();
  const notes = useNotes();
  const recorder = useRecorder();
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const record = () => {
    setError(null);
    api.newVoiceNote().catch((e) => setError(errorText(e)));
  };

  // Open the note that was just saved.
  useEffect(() => {
    if (recorder?.saved) setSelected(recorder.saved);
  }, [recorder?.saved]);

  useEffect(() => {
    if (recorder?.error) setError(recorder.error);
  }, [recorder?.error]);

  useEffect(() => {
    if (!error) return;
    const timer = setTimeout(() => setError(null), 6000);
    return () => clearTimeout(timer);
  }, [error]);

  // ⌥⌘N while Voice Notes is open shows the recording, not an old note.
  useEffect(() => {
    if (intent.action === "new") setSelected(null);
  }, [intent.n]);

  const note = useMemo(() => {
    if (!notes) return null;
    return notes.find((n) => n.id === selected) ?? notes[0] ?? null;
  }, [notes, selected]);

  if (!notes || !recorder) return null;
  const phase = recorder.phase;
  const busy = phase === "recording" || phase === "paused";

  let main: React.ReactNode;
  if (busy) main = <Recording key={recorder.id} s={s} recorder={recorder} />;
  else if (phase === "finishing") main = <Finishing s={s} recorder={recorder} />;
  else if (note) main = <NoteView key={note.id} s={s} note={note} />;
  else main = <Empty s={s} onRecord={record} />;

  return (
    <>
      <ToolSidebar
        label="Voice Notes"
        title="Voice Notes"
        onBack={onBack}
        search={search}
        searchPlaceholder="Search notes"
        action={{ label: "New note (⌥⌘N)", icon: <RecordIcon />, onClick: record }}
      >
        <NoteList
          notes={notes}
          query={search.query}
          selected={busy || phase === "finishing" ? null : note?.id ?? null}
          recorder={recorder}
          onPick={(id) => {
            setSelected(id);
          }}
        />
      </ToolSidebar>
      <div className="relative flex min-w-0 grow">
        {error && <ErrorBar text={error} />}
        {main}
      </div>
    </>
  );
}
