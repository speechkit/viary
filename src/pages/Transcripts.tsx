// Transcripts (TranscriptsEmpty, Transcribe, TranscribeResult artboards):
// audio and video files transcribed in the background, kept apart from
// dictation History.
//
// Speakers, word timing and unsure words show only when recognition
// reports them. speechkit 0.5 reports none, so real transcripts have
// passages with their own times: the player highlights whole passages, and
// subtitles get one cue per passage.

import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open, save } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { Select } from "../components/Controls";
import { Icon } from "../components/Icons";
import { Menu } from "../components/Menu";
import { PlayerControls } from "../components/Player";
import { SidebarEmpty, ToolSidebar, useToolSearch } from "../components/ToolSidebar";
import {
  api,
  clock,
  errorText,
  shortName,
  useJobs,
  useTranscripts,
  type Cue,
  type Job,
  type OutputFormat,
  type Passage,
  type SearchHit,
  type Snapshot,
  type TranscriptDoc,
  type TranscriptSettings,
  type TranscriptSummary,
} from "../lib/ipc";
import { usePlayback, type Playback } from "../lib/playback";
import { speakerColor } from "../lib/speakers";
import { cueTime, currentIndex, daysAgo, matchRanges, spaced } from "../lib/transcript";
import type { Intent } from "../windows/MainWindow";

const lbl = "text-[11px] font-semibold tracking-[0.06em] text-faint uppercase";
const btn = "inline-flex h-[34px] items-center gap-2 whitespace-nowrap rounded-[9px] border border-edge bg-white px-3 text-[13px] font-medium hover:bg-card";
const btnPrimary = "inline-flex h-[34px] items-center gap-2 whitespace-nowrap rounded-[9px] border border-ink bg-ink px-3 text-[13px] font-medium text-white";
const bigBtn = "inline-flex h-10 items-center rounded-[9px] border px-[18px] text-sm font-medium";
/** A word below this confidence gets the dotted "unsure" underline. */
const UNSURE = 0.75;

/** The formats speechkit decodes in this build, for the dialog and the hint. */
function useExtensions(): string[] {
  const [extensions, setExtensions] = useState<string[]>([]);
  useEffect(() => {
    api.audioExtensions().then(setExtensions, console.error);
  }, []);
  return extensions;
}

/** True while files are dragged over the window; dropped paths go to the queue. */
function useWindowDrop(): boolean {
  const [over, setOver] = useState(false);
  useEffect(() => {
    const stop = getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (payload.type === "enter" || payload.type === "over") setOver(true);
      else if (payload.type === "leave") setOver(false);
      else {
        setOver(false);
        if (payload.paths.length) api.transcribeFiles(payload.paths).catch(console.error);
      }
    });
    return () => {
      stop.then((f) => f());
    };
  }, []);
  return over;
}

async function chooseFiles(extensions: string[]) {
  const picked = await open({ multiple: true, filters: [{ name: "Audio and video", extensions }] });
  if (picked?.length) await api.transcribeFiles(picked);
}

async function chooseFolder() {
  const picked = await open({ directory: true, multiple: true });
  if (picked?.length) await api.transcribeFiles(picked);
}

const formatList = (extensions: string[]) => extensions.map((e) => e.toUpperCase()).join(", ");

function when(ms: number): string {
  const date = new Date(ms);
  const days = daysAgo(ms);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return date.toLocaleDateString(undefined, { weekday: "short" });
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

const extOf = (file: string) => file.slice(file.lastIndexOf(".") + 1).toUpperCase();

// ---------------------------------------------------------------------------
// Engines and settings for new files

/** The engines set up for new files: the dictation engine, or another. */
function usableEngines(s: Snapshot): { value: string; label: string }[] {
  const active = s.engine.active;
  const choices = [{ value: "", label: active ? `Same as dictation · ${active.name}` : "Same as dictation" }];
  for (const model of s.settings.localModels) choices.push({ value: `local:${model.id}`, label: shortName(model.name) });
  if (s.keys.openAi && s.settings.openai.model) choices.push({ value: "openai", label: `OpenAI · ${s.settings.openai.model}` });
  if (s.keys.dashScope && s.settings.dashscope.model) choices.push({ value: "dashscope", label: `DashScope · ${s.settings.dashscope.model}` });
  return choices;
}

/** Why the engine chosen for files cannot be used, or null when it can. */
function unusableEngine(s: Snapshot): string | null {
  const chosen = s.settings.transcripts.engine;
  if (!chosen || usableEngines(s).some((c) => c.value === chosen)) return null;
  const cloud = (name: string, hasKey: boolean) => `${name} · ${hasKey ? "choose a model" : "add an API key"}`;
  if (chosen === "openai") return cloud("OpenAI", s.keys.openAi);
  if (chosen === "dashscope") return cloud("DashScope", s.keys.dashScope);
  return "Removed model · choose another";
}

/** The choices for new files. A chosen engine that cannot be used stays
 *  listed, saying why, so it can be seen and changed. */
function engineChoices(s: Snapshot): { value: string; label: string }[] {
  const choices = usableEngines(s);
  const unusable = unusableEngine(s);
  if (unusable) choices.push({ value: s.settings.transcripts.engine!, label: unusable });
  return choices;
}

function engineChoiceLabel(s: Snapshot): string {
  const value = s.settings.transcripts.engine ?? "";
  return engineChoices(s).find((c) => c.value === value)?.label ?? "Same as dictation";
}

const FORMATS: OutputFormat[] = ["srt", "vtt", "txt", "md"];
const LANGUAGES = [
  ["auto", "Detect automatically"],
  ["en", "English"],
  ["zh", "中文"],
] as const;

function update(patch: Partial<TranscriptSettings>) {
  api.updateTranscriptSettings(patch).catch(console.error);
}

const SPEAKERS: { value: TranscriptSettings["speakers"]; label: string }[] = [
  { value: "detect", label: "Detect how many" },
  { value: "two", label: "Exactly 2" },
  { value: "one", label: "Just one" },
];

function NewFileSettings({ s }: { s: Snapshot }) {
  const t = s.settings.transcripts;
  const caps = s.speechCaps;
  return (
    <div className="flex shrink-0 flex-col gap-3 rounded-2xl border border-line bg-white px-5 py-4">
      <span className={lbl}>For new files</span>
      <div className="grid grid-cols-2 gap-x-7 gap-y-3">
        <div className="flex items-center gap-3">
          <span className="grow shrink-0 text-sm">Engine</span>
          <Select shrink label="Engine" value={t.engine ?? ""} options={engineChoices(s)} onChange={(engine) => update({ engine: engine || null })} />
        </div>
        <div className="flex items-center gap-3">
          <span className="grow shrink-0 text-sm">Language</span>
          <Select
            shrink
            label="Language"
            value={t.language}
            options={LANGUAGES.map(([value, label]) => ({ value, label }))}
            onChange={(language) => update({ language })}
          />
        </div>
        <div className="flex items-center gap-3" title={caps.speakers ? undefined : "Needs speaker separation, which speechkit does not provide yet"}>
          <span className={"grow text-sm " + (caps.speakers ? "" : "text-faint")}>Speakers</span>
          {caps.speakers ? (
            <Select label="Speakers" value={t.speakers} options={SPEAKERS} onChange={(speakers) => update({ speakers })} />
          ) : (
            <Select label="Speakers" value="off" options={[{ value: "off", label: "Not available yet" }]} disabled onChange={() => {}} />
          )}
        </div>
        <div className="flex items-center gap-3">
          <span className="grow text-sm">Save automatically</span>
          <div role="group" aria-label="Output formats" className="flex gap-1.5">
            {FORMATS.map((f) => {
              const on = t.formats.includes(f);
              return (
                <button
                  key={f}
                  type="button"
                  aria-pressed={on}
                  onClick={() => update({ formats: on ? t.formats.filter((x) => x !== f) : FORMATS.filter((x) => x === f || t.formats.includes(x)) })}
                  className={
                    "inline-flex h-[30px] items-center rounded-lg border px-2.5 font-mono text-[13px] font-medium " +
                    (on ? "border-ink bg-ink text-white" : "border-edge bg-white text-ink")
                  }
                >
                  {f.toUpperCase()}
                </button>
              );
            })}
          </div>
        </div>
      </div>
      <span className="text-xs leading-[1.45] text-muted">
        Saved next to the original with the same name, so video players pick up the subtitles. A file already there is kept; Viary saves its
        own copy beside it. Viary keeps the transcript and your edits, and links to the original file instead of copying it.
      </span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Sidebar

function JobRow({ job }: { job: Job }) {
  const state = job.state;
  return (
    <div className="mx-2.5 mb-0.5 flex flex-col gap-[5px] rounded-[10px] px-3 py-2.5">
      <div className="flex justify-between gap-2">
        <span className="truncate text-[13px] font-semibold" title={job.path}>
          {job.name}
        </span>
        <span className="shrink-0 font-mono text-xs text-faint">{job.durationMs !== null ? clock(job.durationMs) : ""}</span>
      </div>
      {state.kind === "running" && state.progress !== null && (
        <div role="progressbar" aria-label={`${job.name} progress`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(state.progress * 100)} className="h-1 overflow-hidden rounded-full bg-rule">
          <div className="h-full bg-ink transition-[width] duration-500" style={{ width: `${state.progress * 100}%` }} />
        </div>
      )}
      <div className="flex justify-between gap-2 text-xs">
        <span className={state.kind === "failed" ? "text-rust" : state.kind === "running" ? "text-muted" : "text-faint"}>
          {state.kind === "waiting" && "Waiting"}
          {state.kind === "running" && state.stage + (state.secondsLeft !== null ? ` · ${minutesLeft(state.secondsLeft)}` : "")}
          {state.kind === "failed" && state.error}
        </span>
        <span className="flex shrink-0 gap-2.5">
          {state.kind === "failed" && (
            <button type="button" onClick={() => api.retryJob(job.id).catch(console.error)} className="font-semibold text-blue">
              Retry
            </button>
          )}
          <button type="button" onClick={() => api.cancelJob(job.id).catch(console.error)} className="font-semibold text-faint hover:text-ink">
            {state.kind === "failed" ? "Remove" : "Cancel"}
          </button>
        </span>
      </div>
    </div>
  );
}

function minutesLeft(seconds: number): string {
  if (seconds < 60) return "less than a minute left";
  return `${Math.round(seconds / 60)} min left`;
}

function LibraryRow({ t, current, onPick }: { t: TranscriptSummary; current: boolean; onPick: () => void }) {
  const n = t.speakers.length;
  const meta = [when(t.createdAt), n ? `${n} ${n === 1 ? "speaker" : "speakers"}` : null, t.saved.length ? t.saved.map(extOf).join(", ") : null]
    .filter(Boolean)
    .join(" · ");
  return (
    <button
      type="button"
      onClick={onPick}
      aria-current={current ? "page" : undefined}
      className={"mx-2.5 mb-0.5 flex flex-col gap-[3px] rounded-[10px] px-3 py-2.5 text-left " + (current ? "bg-white" : "hover:bg-white/60")}
    >
      <span className="flex w-full justify-between gap-2">
        <span className="truncate text-[13px] font-semibold">{t.name}</span>
        <span className="shrink-0 font-mono text-xs text-faint">{clock(t.durationMs)}</span>
      </span>
      <span className="text-xs text-muted">{meta}</span>
    </button>
  );
}

function SearchResults({ query, onPick }: { query: string; onPick: (id: string, atMs: number) => void }) {
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  useEffect(() => {
    const q = query.trim();
    if (!q) return setHits(null);
    const timer = setTimeout(() => api.searchTranscripts(q).then(setHits, console.error), 150);
    return () => clearTimeout(timer);
  }, [query]);
  if (!hits) return null;
  const total = hits.reduce((n, h) => n + h.count, 0);
  return (
    <div className="scroll flex grow flex-col pb-4">
      <span className={lbl + " px-5 pt-2.5 pb-1.5"}>
        {hits.length} {hits.length === 1 ? "transcript" : "transcripts"} · {total} {total === 1 ? "match" : "matches"}
      </span>
      {hits.map((h) => (
        <button key={h.id} type="button" onClick={() => onPick(h.id, h.atMs)} className="mx-2.5 mb-0.5 flex flex-col gap-1 rounded-[10px] px-3 py-2.5 text-left hover:bg-white/60">
          <span className="flex w-full justify-between gap-2">
            <span className="truncate text-[13px] font-semibold">{h.name}</span>
            <span className="shrink-0 text-xs text-faint">
              {h.count} {h.count === 1 ? "match" : "matches"}
            </span>
          </span>
          <span className="text-xs leading-[1.45] text-muted">
            <span className="font-mono text-faint">{clock(h.atMs)}</span> {h.before}
            <mark className="rounded-[2px] bg-[#FFE3D3] px-px text-ink">{h.match}</mark>
            {h.after}
          </span>
        </button>
      ))}
      <span className="px-5 py-3 text-xs text-faint">{hits.length ? "Esc clears the search and shows everything again." : `Nothing mentions “${query.trim()}”.`}</span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Empty, and adding files

function DropHint({ s, extensions, compact }: { s: Snapshot; extensions: string[]; compact?: boolean }) {
  return (
    <>
      <div className="flex gap-2">
        <button type="button" onClick={() => chooseFiles(extensions).catch(console.error)} className={(compact ? btnPrimary : bigBtn + " border-ink bg-ink text-white")}>
          Choose files…
        </button>
        <button type="button" onClick={() => chooseFolder().catch(console.error)} className={compact ? btn : bigBtn + " border-edge bg-white text-ink"}>
          Add a folder
        </button>
      </div>
      <span className={compact ? "text-[13px] leading-[1.45] text-muted" : "text-xs text-faint"}>
        {formatList(extensions)} · up to 3 hours · or drop on the Viary icon in the menu bar
        {!s.engine.active && !s.settings.transcripts.engine && " · choose a voice engine first"}
        {unusableEngine(s) && " · the engine for files can't be used; choose another below"}
      </span>
    </>
  );
}

function Empty({ s, extensions, over, onChange }: { s: Snapshot; extensions: string[]; over: boolean; onChange: () => void }) {
  const t = s.settings.transcripts;
  return (
    <main className="flex grow px-12 py-10">
      <div
        className={
          "flex grow flex-col items-center justify-center gap-6 rounded-[22px] border-2 border-dashed p-10 text-center transition-colors " +
          (over ? "border-blue bg-blue-wash" : "border-stone bg-card")
        }
      >
        <div aria-hidden="true" className="flex size-28 items-center justify-center rounded-[28px] bg-white text-ink shadow-[0_14px_34px_rgba(28,27,24,0.14),inset_0_0_0_2px_#E7E1D5]">
          <svg viewBox="0 0 24 24" width="48" height="48" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round">
            <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8zM14 3v5h5" />
            <path d="M9 13v3M12 11v7M15 13v3" stroke="#E0452B" />
          </svg>
        </div>
        <div className="flex flex-col items-center gap-2.5">
          <h2 className="m-0 font-serif text-4xl font-normal">Transcribe your first file</h2>
          <p className="m-0 max-w-[500px] text-[15px] leading-[1.6] text-muted">
            Drop a meeting, interview, podcast or video here. Viary writes down what was said
            {s.speechCaps.speakers ? ", finds who spoke when" : ""}, times {s.speechCaps.wordTimings ? "every word" : "each passage"}, and saves subtitles next to the file.
          </p>
        </div>
        <DropHint s={s} extensions={extensions} />
        <div className="flex items-center gap-2.5 rounded-xl border border-line bg-white px-3.5 py-2.5 text-[13px] text-muted">
          <span>New files use</span>
          <b className="font-semibold text-ink">{engineChoiceLabel(s)}</b>
          <span>·</span>
          <span>{t.language === "auto" ? "detect language" : LANGUAGES.find(([v]) => v === t.language)?.[1]}</span>
          {s.speechCaps.speakers && (
            <>
              <span>·</span>
              <span>detect speakers</span>
            </>
          )}
          <span>·</span>
          <span>{t.formats.length ? `save ${t.formats.map((f) => f.toUpperCase()).join(" + ")}` : "save nothing next to the file"}</span>
          <button type="button" onClick={onChange} className="pl-1 font-semibold text-blue">
            Change
          </button>
        </div>
      </div>
    </main>
  );
}

function AddFiles({ s, extensions, over }: { s: Snapshot; extensions: string[]; over: boolean }) {
  return (
    <main className="scroll flex w-0 grow flex-col gap-5 px-12 pt-11 pb-9">
      <div className="flex flex-col gap-1.5">
        <h2 className="m-0 font-serif text-[34px] font-normal">Add something to transcribe</h2>
        <span className="text-[13px] leading-[1.45] text-muted">
          Meetings, interviews, podcasts, videos. Viary {s.speechCaps.speakers ? "finds who spoke when and " : ""}times{" "}
          {s.speechCaps.wordTimings ? "every word" : "each passage"}. Jobs run in the background, so you can keep dictating.
        </span>
      </div>
      <div
        className={
          "flex min-h-[220px] grow flex-col items-center justify-center gap-3 rounded-[18px] border-2 border-dashed p-6 text-center transition-colors " +
          (over ? "border-blue bg-blue-wash" : "border-stone bg-card")
        }
      >
        <Icon name="upload" size={36} className="shrink-0 text-muted" />
        <span className="text-lg font-semibold">Drop audio or video here</span>
        <DropHint s={s} extensions={extensions} compact />
      </div>
      <NewFileSettings s={s} />
    </main>
  );
}

// ---------------------------------------------------------------------------
// The editor

/** Words of the original that the edit changed, by a word-level diff. */
function changedWords(original: string, text: string): boolean[] {
  const a = original.split(/\s+/).filter(Boolean);
  const b = text.split(/\s+/).filter(Boolean);
  // Longest common subsequence; b's words outside it were edited.
  const dp = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--)
    for (let j = b.length - 1; j >= 0; j--) dp[i][j] = a[i] === b[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
  const changed = new Array<boolean>(b.length).fill(true);
  for (let i = 0, j = 0; i < a.length && j < b.length; ) {
    if (a[i] === b[j]) {
      changed[j] = false;
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) i++;
    else j++;
  }
  return changed;
}

/** Splits `text` around case-insensitive matches of `query`. */
function highlight(text: string, query: string): React.ReactNode {
  const ranges = matchRanges(text, query);
  if (!ranges.length) return text;
  const parts: React.ReactNode[] = [];
  let at = 0;
  for (const [start, end] of ranges) {
    parts.push(text.slice(at, start), <mark key={start} className="rounded-[2px] bg-[#FFE3D3] text-ink">{text.slice(start, end)}</mark>);
    at = end;
  }
  parts.push(text.slice(at));
  return parts;
}


function PassageText({ p, pos, active, find, onSeek }: { p: Passage; pos: number; active: boolean; find: string; onSeek: (ms: number) => void }) {
  if (p.original && p.original !== p.text) {
    // Edited: timing stays with the passage; the changed words are marked.
    const changed = changedWords(p.original, p.text);
    return (
      <>
        {p.text
          .split(/\s+/)
          .filter(Boolean)
          .map((w, i) => (
            <span key={i}>
              {i > 0 ? " " : ""}
              {changed[i] ? (
                <span className="rounded-[3px] bg-[#EEF0FB] px-0.5" title="You edited this; its timing stays with the passage">
                  {highlight(w, find)}
                </span>
              ) : (
                highlight(w, find)
              )}
            </span>
          ))}
      </>
    );
  }
  if (!p.words) return <>{highlight(p.text, find)}</>;
  const now = active ? p.words.findIndex((w) => pos >= w.startMs && pos < w.endMs) : -1;
  const spoken = active ? p.words.filter((w) => w.endMs <= pos).length - 1 : p.words.length;
  return (
    <>
      {p.words.map((w, i) => {
        const unsure = w.confidence !== null && w.confidence < UNSURE;
        return (
          <span key={i}>
            {i > 0 && spaced(p.words![i - 1].text, w.text) ? " " : ""}
            <span
              data-unsure={unsure ? w.startMs : undefined}
              onClick={unsure ? () => onSeek(w.startMs) : undefined}
              title={unsure ? "Unsure: click to hear it" : undefined}
              className={
                (i === now ? "rounded-[3px] bg-[#FFE3D3] shadow-[0_0_0_2px_#FFE3D3] " : active && i > spoken ? "text-[#8A847A] " : "") +
                (unsure ? "cursor-pointer underline decoration-[#B7791F] decoration-dotted decoration-2 underline-offset-[3px]" : "")
              }
            >
              {highlight(w.text, find)}
            </span>
          </span>
        );
      })}
    </>
  );
}

function SpeakerName({ doc, index }: { doc: TranscriptDoc; index: number }) {
  const [draft, setDraft] = useState<string | null>(null);
  const name = doc.speakers[index]?.name ?? "";
  const generic = /^Speaker \d+$/.test(name);
  const color = speakerColor(index);
  if (draft !== null) {
    const done = () => {
      if (draft.trim() && draft.trim() !== name) api.renameSpeaker(doc.id, index, draft.trim()).catch(console.error);
      setDraft(null);
    };
    return (
      <input
        autoFocus
        aria-label="Speaker name"
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={done}
        onKeyDown={(e) => {
          if (e.key === "Enter") done();
          if (e.key === "Escape") setDraft(null);
        }}
        className="w-40 border-0 border-b border-current bg-transparent p-0 text-[13px] font-semibold outline-0"
        style={{ color }}
      />
    );
  }
  return (
    <button type="button" onClick={() => setDraft(name)} title="Rename this speaker everywhere" className="group/name inline-flex items-center gap-1.5 text-[13px] font-semibold" style={{ color }}>
      {name}
      {generic ? <span className="text-xs font-normal text-faint">· name</span> : <Icon name="pen" size={12} className="opacity-0 group-hover/name:opacity-100" />}
    </button>
  );
}

function PassageEditor({ doc, index, onDone }: { doc: TranscriptDoc; index: number; onDone: () => void }) {
  const [text, setText] = useState(doc.passages[index].text);
  const area = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    const el = area.current;
    if (!el) return;
    el.style.height = "0";
    el.style.height = `${el.scrollHeight}px`;
  }, [text]);
  const save = () => {
    if (text.trim() && text !== doc.passages[index].text) api.editPassage(doc.id, index, text.trim()).catch(console.error);
    onDone();
  };
  return (
    <textarea
      ref={area}
      autoFocus
      aria-label="Edit passage"
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={save}
      onKeyDown={(e) => {
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          save();
        }
        if (e.key === "Escape") onDone();
      }}
      className="-mx-1.5 block w-[calc(100%+12px)] resize-none overflow-hidden rounded-md border-0 bg-white px-1.5 py-0 text-[15px] leading-[1.65] shadow-[0_0_0_1px_var(--color-blue)] outline-0"
    />
  );
}

function Timeline({ doc, playback }: { doc: TranscriptDoc; playback: Playback }) {
  const lanes = doc.speakers.length
    ? doc.speakers.map((sp, i) => ({ name: sp.name, color: speakerColor(i), passages: doc.passages.filter((p) => p.speaker === i) }))
    : [{ name: "Speech", color: "#1C1B18", passages: doc.passages }];
  const pct = (ms: number) => `${(ms / Math.max(1, doc.durationMs)) * 100}%`;
  return (
    <div aria-label="Speaker timeline" className="mx-8 flex shrink-0 flex-col gap-1.5 rounded-[14px] border border-line bg-white px-4 py-3">
      {lanes.map((lane) => (
        <div key={lane.name} className="flex h-[22px] items-center gap-3">
          <span className="flex w-[110px] shrink-0 items-center gap-1.5 truncate text-xs font-semibold" style={{ color: lane.color }}>
            <span className="size-2 shrink-0 rounded-full" style={{ background: lane.color }} />
            {lane.name}
          </span>
          <button
            type="button"
            aria-label={`Jump in ${lane.name}'s lane`}
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect();
              playback.seek(((e.clientX - r.left) / r.width) * doc.durationMs, playback.playing);
            }}
            className="relative h-2.5 grow rounded-[3px] bg-paper"
          >
            {lane.passages.map((p, i) => (
              <span key={i} className="absolute inset-y-0 rounded-[3px]" style={{ left: pct(p.startMs), width: pct(Math.max(p.endMs - p.startMs, doc.durationMs / 400)), background: lane.color }} />
            ))}
            <span aria-hidden="true" className="absolute -inset-y-1.5 w-0.5 bg-ink" style={{ left: pct(playback.pos) }} />
          </button>
        </div>
      ))}
    </div>
  );
}

function Subtitles({ s, doc, playback }: { s: Snapshot; doc: TranscriptDoc; playback: Playback }) {
  const [format, setFormat] = useState<"srt" | "vtt">("srt");
  const [cues, setCues] = useState<Cue[]>([]);
  const names = s.settings.transcripts.speakerNames;
  const rows = useRef<(HTMLDivElement | null)[]>([]);
  useEffect(() => {
    api.cues(doc.id, names).then(setCues, console.error);
  }, [doc, names]);
  const current = cues.findIndex((c) => playback.pos >= c.startMs && playback.pos < c.endMs);
  useEffect(() => {
    if (playback.playing && current >= 0) rows.current[current]?.scrollIntoView({ block: "nearest" });
  }, [current, playback.playing]);

  return (
    <aside aria-label="Subtitles" className="flex w-[330px] shrink-0 flex-col overflow-hidden rounded-[14px] border border-line bg-white">
      <div className="flex items-center justify-between border-b border-sand px-3.5 py-3">
        <span className={lbl}>Subtitles · {cues.length} cues</span>
        <div role="group" aria-label="Subtitle format" className="inline-flex gap-0.5 rounded-lg bg-sand p-[3px]">
          {(["srt", "vtt"] as const).map((f) => (
            <button
              key={f}
              type="button"
              aria-pressed={format === f}
              onClick={() => setFormat(f)}
              className={"h-[26px] rounded-md px-2.5 font-mono text-xs font-medium " + (format === f ? "bg-white text-ink shadow-[0_1px_2px_rgba(0,0,0,.12)]" : "text-muted")}
            >
              {f.toUpperCase()}
            </button>
          ))}
        </div>
      </div>
      <div className="scroll flex grow flex-col">
        {format === "vtt" && <span className="border-b border-hair px-3.5 py-2.5 font-mono text-[11px] text-faint">WEBVTT</span>}
        {cues.map((c, i) => (
          <div
            key={c.n}
            ref={(el) => {
              rows.current[i] = el;
            }}
            onClick={() => playback.seek(c.startMs)}
            className="flex cursor-pointer flex-col gap-[3px] border-b border-hair px-3.5 py-2.5"
            style={{ background: i === current ? "#FFF4EC" : "transparent" }}
          >
            <span className="font-mono text-[11px] text-faint">
              {format === "srt" ? `${c.n}  ` : ""}
              {cueTime(c.startMs, format)} → {cueTime(c.endMs, format)}
            </span>
            <span className="text-[13px] leading-[1.45] whitespace-pre-line">{c.text}</span>
            {c.warning && <span className="text-[11px] text-[#8A5A00]">{c.warning}</span>}
          </div>
        ))}
      </div>
      <div className="flex flex-col gap-1.5 border-t border-sand px-3.5 py-2.5 text-xs text-muted">
        <span>{s.speechCaps.wordTimings ? "Up to 42 characters × 2 lines · 1–7 s per cue" : "One cue per passage, since words are not timed yet"}</span>
        {doc.speakers.length > 0 && (
          <label className="flex items-center gap-2">
            <input type="checkbox" checked={names} onChange={(e) => update({ speakerNames: e.target.checked })} />
            Start each cue with the speaker's name
          </label>
        )}
      </div>
    </aside>
  );
}

function FindReplace({ doc, find, setFind, onClose }: { doc: TranscriptDoc; find: string; setFind: (q: string) => void; onClose: () => void }) {
  const [replace, setReplace] = useState("");
  const [done, setDone] = useState<string | null>(null);
  const q = find.trim().toLowerCase();
  const count = q ? doc.passages.reduce((n, p) => n + matchRanges(p.text, find).length, 0) : 0;
  const input = "h-8 w-56 rounded-lg border border-edge bg-white px-2.5 text-[13px] outline-0 focus:border-blue";
  return (
    <div className="mx-8 mb-3 flex shrink-0 items-center gap-2 rounded-xl border border-line bg-white px-3 py-2">
      <input autoFocus aria-label="Find" placeholder="Find" value={find} onChange={(e) => setFind(e.target.value)} onKeyDown={(e) => e.key === "Escape" && onClose()} className={input} />
      <input aria-label="Replace with" placeholder="Replace with" value={replace} onChange={(e) => setReplace(e.target.value)} onKeyDown={(e) => e.key === "Escape" && onClose()} className={input} />
      <button
        type="button"
        disabled={!count}
        onClick={() =>
          api.replaceAll(doc.id, find.trim(), replace).then((n) => setDone(`Replaced ${n}`), (e) => setDone(errorText(e)))
        }
        className={btn + " h-8 disabled:opacity-50"}
      >
        Replace all
      </button>
      <span className="grow text-xs text-faint">{done ?? (q ? `${count} ${count === 1 ? "match" : "matches"}` : "")}</span>
      <button type="button" onClick={onClose} aria-label="Close find and replace" className="inline-flex size-7 items-center justify-center rounded-md text-muted hover:bg-sand">
        <Icon name="close" size={14} />
      </button>
    </div>
  );
}

function Editor({ s, doc, seekTo }: { s: Snapshot; doc: TranscriptDoc; seekTo: number | null }) {
  const playback = usePlayback(doc.sourceExists ? doc.source : null, doc.durationMs);
  const [editing, setEditing] = useState<number | null>(null);
  const [finding, setFinding] = useState(false);
  const [find, setFind] = useState("");
  const rows = useRef<(HTMLDivElement | null)[]>([]);
  const article = useRef<HTMLElement>(null);

  const current = currentIndex(doc.passages, playback.pos);
  const started = playback.playing || playback.pos > 0;
  const hasUnsure = doc.passages.some((p) => !p.original && p.words?.some((w) => w.confidence !== null && w.confidence < UNSURE));

  // From a search result: show the match.
  useEffect(() => {
    if (seekTo === null) return;
    playback.seek(seekTo, false);
    const i = doc.passages.findIndex((p) => p.endMs > seekTo);
    rows.current[i]?.scrollIntoView({ block: "center" });
  }, [seekTo, doc.id]);

  useEffect(() => {
    if (playback.playing && current >= 0 && editing === null) rows.current[current]?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [current, playback.playing]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement).tagName;
      if (["INPUT", "TEXTAREA", "SELECT"].includes(tag)) return;
      if (e.key === " " && tag !== "BUTTON") {
        e.preventDefault();
        playback.toggle();
      }
      if (e.metaKey && e.altKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        setFinding(true);
      }
      // Tab jumps to the next unsure word after the playhead.
      if (e.key === "Tab" && hasUnsure && !e.metaKey) {
        const marks = [...(article.current?.querySelectorAll<HTMLElement>("[data-unsure]") ?? [])];
        const next = marks.find((m) => Number(m.dataset.unsure) > playback.pos + 50) ?? marks[0];
        if (!next) return;
        e.preventDefault();
        playback.seek(Number(next.dataset.unsure), false);
        next.scrollIntoView({ block: "center" });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [playback, hasUnsure]);

  const exportAs = async (format: OutputFormat) => {
    const path = await save({ defaultPath: `${doc.name}.${format}`, filters: [{ name: format.toUpperCase(), extensions: [format] }] });
    if (path) await api.exportTranscript(doc.id, format, path);
  };

  const engine = `${doc.engine.kind} ${doc.engine.onDevice ? "on-device" : "in the cloud"}`;
  const caption = [
    clock(doc.durationMs),
    doc.speakers.length ? `${doc.speakers.length} ${doc.speakers.length === 1 ? "speaker" : "speakers"}` : null,
    engine,
    doc.saved.length ? `saved ${doc.saved.join(" and ")}` : null,
    doc.sourceExists ? null : "the original file has moved",
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <main className="flex w-0 grow flex-col">
      <div data-tauri-drag-region className="flex items-end gap-4 px-8 pt-10 pb-3.5">
        <div className="flex min-w-0 grow flex-col gap-1">
          <h1 className="m-0 truncate font-serif text-[28px] font-normal">{doc.name}</h1>
          <span className="truncate text-xs text-faint" title={doc.source}>
            {caption}
          </span>
        </div>
        <button type="button" aria-pressed={finding} onClick={() => setFinding(!finding)} title="Find & replace (⌥⌘F)" className={btn}>
          <Icon name="search" />
          Find &amp; replace
        </button>
        <Menu
          label="Export"
          className={btnPrimary}
          items={[
            ...(["srt", "vtt", "txt", "md"] as const).map((f) => ({
              label: { srt: "Subtitles (.srt)", vtt: "Subtitles (.vtt)", txt: "Plain text (.txt)", md: "Markdown (.md)" }[f],
              onSelect: () => exportAs(f).catch(console.error),
            })),
            { label: "Delete transcript", danger: true, onSelect: () => api.deleteTranscript(doc.id).catch(console.error) },
          ]}
        >
          Export ▾
        </Menu>
      </div>
      {finding && <FindReplace doc={doc} find={find} setFind={setFind} onClose={() => (setFinding(false), setFind(""))} />}
      <Timeline doc={doc} playback={playback} />
      <div className="flex min-h-0 grow gap-5 px-8 pt-4">
        <article ref={article} aria-label="Transcript" className="scroll flex w-0 grow flex-col gap-1 pb-4 text-[15px] leading-[1.65]">
          {doc.passages.length === 0 && <p className="m-0 text-faint">Nothing was recognized in this file.</p>}
          {doc.passages.map((p, i) => {
            const isNow = i === current && started;
            return (
              <div
                key={i}
                ref={(el) => {
                  rows.current[i] = el;
                }}
                className="-mx-3 flex gap-3 rounded-[10px] px-3 py-2 transition-colors duration-300"
                style={{ background: isNow ? "#FBFAF7" : "transparent" }}
              >
                <button type="button" onClick={() => playback.seek(p.startMs)} aria-label={`Play from ${clock(p.startMs)}`} className="w-16 shrink-0 self-start pt-[3px] text-left font-mono text-xs text-faint hover:text-blue hover:underline">
                  {clock(p.startMs)}
                </button>
                <div className="min-w-0 grow">
                  {p.speaker !== null && doc.speakers[p.speaker] && <SpeakerName doc={doc} index={p.speaker} />}
                  {editing === i ? (
                    <PassageEditor doc={doc} index={i} onDone={() => setEditing(null)} />
                  ) : (
                    <p
                      onDoubleClick={() => setEditing(i)}
                      title="Double-click to edit"
                      className={"selectable m-0 cursor-text " + (started && i > current ? "text-[#8A847A]" : "")}
                    >
                      <PassageText p={p} pos={playback.pos} active={isNow} find={finding ? find : ""} onSeek={(ms) => playback.seek(ms)} />
                    </p>
                  )}
                </div>
              </div>
            );
          })}
        </article>
        <Subtitles s={s} doc={doc} playback={playback} />
      </div>
      <div className="mx-6 mt-3.5 mb-[18px] rounded-[14px] border border-line bg-white px-3.5 py-2">
        <PlayerControls playback={playback} durationMs={doc.durationMs}>
          {!doc.sourceExists && <span className="text-xs text-faint">The original file is no longer where it was.</span>}
          {hasUnsure && (
            <span className="text-xs text-faint">
              <span className="underline decoration-[#B7791F] decoration-dotted decoration-2 underline-offset-[3px]">dotted</span> words: unsure, click to check · Tab
              jumps to the next one
            </span>
          )}
        </PlayerControls>
      </div>
    </main>
  );
}

function useDoc(id: string | null): TranscriptDoc | null {
  const [doc, setDoc] = useState<TranscriptDoc | null>(null);
  useEffect(() => {
    if (!id) return setDoc(null);
    let live = true;
    const load = () => api.transcript(id).then((d) => live && setDoc(d), () => live && setDoc(null));
    load();
    const stop = listen("transcripts-changed", load);
    return () => {
      live = false;
      stop.then((f) => f());
    };
  }, [id]);
  return doc && doc.id === id ? doc : null;
}

// ---------------------------------------------------------------------------

export function TranscriptsPage({ s, onBack, intent }: { s: Snapshot; onBack: () => void; intent: Intent }) {
  const search = useToolSearch();
  const extensions = useExtensions();
  const over = useWindowDrop();
  const jobs = useJobs();
  const library = useTranscripts();
  const [selected, setSelected] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [seekTo, setSeekTo] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const doc = useDoc(selected);

  const add = () => chooseFiles(extensions).catch((e) => setError(errorText(e)));

  // "Transcribe a file…" in the menu bar opens the picker here.
  useEffect(() => {
    if (intent.action === "choose" && extensions.length) add();
    // Only a new request opens it, not the extensions arriving later.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [intent.n, extensions.length > 0]);

  // Files just added: show the queue, not an old transcript. Files that
  // could not be added (dropped on the window or the menu bar icon): say so.
  useEffect(() => {
    if (intent.action === "added") setSelected(null);
    if (intent.action === "nothing") setError("None of these files can be transcribed");
  }, [intent.n]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && e.key.toLowerCase() === "o") {
        e.preventDefault();
        add();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // A deleted transcript: back to adding files.
  useEffect(() => {
    if (selected && library && !library.some((t) => t.id === selected)) setSelected(null);
  }, [library, selected]);

  const pick = (id: string, at: number | null = null) => {
    setSelected(id);
    setAdding(false);
    setSeekTo(at);
  };

  const searching = search.open && search.query.trim().length > 0;
  const empty = library !== null && library.length === 0 && jobs.length === 0;

  let main: React.ReactNode;
  if (selected && doc && !adding) main = <Editor key={doc.id} s={s} doc={doc} seekTo={seekTo} />;
  else if (empty && !adding) main = <Empty s={s} extensions={extensions} over={over} onChange={() => setAdding(true)} />;
  else main = <AddFiles s={s} extensions={extensions} over={over} />;

  return (
    <>
      <ToolSidebar
        label="Transcripts"
        title="Transcripts"
        onBack={onBack}
        search={search}
        searchPlaceholder="Search transcripts"
        action={{ label: "Add files (⌘O)", icon: <Icon name="fileAdd" size={19} strokeWidth={1.7} />, onClick: add }}
      >
        {searching ? (
          <SearchResults query={search.query} onPick={pick} />
        ) : empty ? (
          <SidebarEmpty icon={<Icon name="transcript" size={28} strokeWidth={1.5} />} title="No transcripts yet" text="Files you add show their progress here, then stay in your library." />
        ) : (
          <div className="scroll flex grow flex-col pb-4">
            {jobs.length > 0 && (
              <>
                <span className={lbl + " px-5 pt-2.5 pb-1.5"}>In progress · {jobs.length}</span>
                {jobs.map((job) => (
                  <JobRow key={job.id} job={job} />
                ))}
              </>
            )}
            {library && library.length > 0 && (
              <>
                <span className={lbl + " px-5 pt-2.5 pb-1.5"}>Library</span>
                {library.map((t) => (
                  <LibraryRow key={t.id} t={t} current={t.id === selected && !adding} onPick={() => pick(t.id)} />
                ))}
              </>
            )}
          </div>
        )}
      </ToolSidebar>
      <div className="relative flex min-w-0 grow">
        {error && (
          <div role="alert" onClick={() => setError(null)} className="absolute top-3 left-1/2 z-20 -translate-x-1/2 rounded-[10px] bg-rust-wash px-4 py-2 text-[13px] text-rust">
            {error}
          </div>
        )}
        {main}
      </div>
    </>
  );
}
