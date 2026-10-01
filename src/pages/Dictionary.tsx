// Dictionary (Dictionary artboard): names, jargon and spellings the engine
// should get right, as hotwords, a prompt, or replacements after recognition.

import { useState } from "react";
import { btn, btnPrimary, ErrorBanner, input, Seg } from "../components/Controls";
import { Icon } from "../components/Icons";
import {
  api,
  errorText,
  knownApps,
  MAX_WORDS,
  useHistory,
  type DictionaryEntry,
  type Snapshot,
} from "../lib/ipc";

type Filter = "all" | "person" | "term" | "replacement";

const FILTERS: [Filter, string][] = [
  ["all", "All"],
  ["person", "People"],
  ["term", "Terms"],
  ["replacement", "Replacements"],
];

function matches(entry: DictionaryEntry, filter: Filter): boolean {
  if (filter === "all") return true;
  if (filter === "replacement") return entry.soundsLike.length > 0;
  return entry.kind === filter;
}

const EMPTY: DictionaryEntry = { id: "", word: "", soundsLike: [], kind: "term", boost: "normal", apps: [] };

/** Splits "may lin, mei lynn" into its phrases. */
function phrases(text: string): string[] {
  return text
    .split(/[,，]/)
    .map((p) => p.trim())
    .filter(Boolean);
}

/** What the active engine does with the dictionary, under the table. */
function engineNote(s: Snapshot): string {
  const active = s.engine.active;
  if (!active) return "Choose a voice engine to use the dictionary.";
  switch (active.dictionary) {
    case "hotwords":
      return `${active.kind}: words are used as hotwords while you speak, and “When I say” phrases are replaced after recognition.`;
    case "prompt":
      return `${active.kind}: words are given to the engine as context, and “When I say” phrases are replaced after recognition.`;
    default:
      return `${active.kind} does not support recognition hotwords. To correct a mishearing, enter the engine’s incorrect spelling under “When I say”; “Write as” alone only restores capitalization. For example: “通易千问” → “通义千问”.`;
  }
}

function AppPicker({
  apps,
  suggestions,
  onChange,
}: {
  apps: string[];
  suggestions: string[];
  onChange: (apps: string[]) => void;
}) {
  const [typed, setTyped] = useState("");
  const [only, setOnly] = useState(apps.length > 0);
  const choices = [...new Set([...apps, ...suggestions])];
  const toggle = (app: string) => onChange(apps.includes(app) ? apps.filter((a) => a !== app) : [...apps, app]);
  const addTyped = () => {
    const app = typed.trim();
    if (app && !apps.includes(app)) onChange([...apps, app]);
    setTyped("");
  };
  return (
    <div className="flex flex-col gap-2.5">
      <div>
        <Seg
          label="Where it applies"
          value={only ? "only" : "all"}
          options={[
            ["all", "All apps"],
            ["only", "Only in…"],
          ]}
          onChange={(v) => {
            setOnly(v === "only");
            if (v === "all") onChange([]);
          }}
        />
      </div>
      {only && (
        <div className="flex flex-wrap items-center gap-1.5">
          {choices.map((app) => {
            const on = apps.includes(app);
            return (
              <button
                key={app}
                type="button"
                aria-pressed={on}
                onClick={() => toggle(app)}
                className={
                  "inline-flex h-7 items-center gap-1.5 rounded-full border px-2.5 text-[13px] " +
                  (on ? "border-ink bg-ink text-white" : "border-edge bg-white text-ink hover:border-stone")
                }
              >
                {on && <Icon name="check" size={12} />}
                {app}
              </button>
            );
          })}
          <input
            className={`${input} h-7 w-[150px] rounded-full`}
            aria-label="Add an app"
            placeholder="Another app…"
            spellCheck={false}
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                addTyped();
              }
            }}
            onBlur={addTyped}
          />
        </div>
      )}
    </div>
  );
}

function Editor({
  entry,
  apps,
  replacementsOnly,
  onDone,
  onError,
}: {
  entry: DictionaryEntry;
  apps: string[];
  replacementsOnly: boolean;
  onDone: () => void;
  onError: (e: string) => void;
}) {
  const [draft, setDraft] = useState(entry);
  const [say, setSay] = useState(entry.soundsLike.join(", "));
  const [busy, setBusy] = useState(false);
  const save = async () => {
    setBusy(true);
    try {
      await api.saveWord({ ...draft, word: draft.word.trim(), soundsLike: phrases(say) });
      onDone();
    } catch (e) {
      onError(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <form
      className="flex flex-col gap-4 rounded-[14px] border-2 border-ink bg-white px-5 py-[18px]"
      onSubmit={(e) => {
        e.preventDefault();
        if (draft.word.trim()) save();
      }}
      onKeyDown={(e) => e.key === "Escape" && onDone()}
    >
      <div className="grid grid-cols-2 gap-4">
        <label className="flex flex-col gap-1.5 text-[13px]">
          <span className="font-medium">Write as</span>
          <input
            className={input}
            autoFocus
            spellCheck={false}
            placeholder="Mei Lin, sherpa-onnx, 通义千问"
            value={draft.word}
            onChange={(e) => setDraft({ ...draft, word: e.target.value })}
          />
        </label>
        <label className="flex flex-col gap-1.5 text-[13px]">
          <span className="font-medium">
            When I say <span className="font-normal text-faint">· {replacementsOnly ? "needed for corrections" : "optional"}</span>
          </span>
          <input
            className={input}
            spellCheck={false}
            placeholder="Incorrect spelling, e.g. 通易千问, may lin"
            value={say}
            onChange={(e) => setSay(e.target.value)}
          />
          <span className="text-xs leading-[1.45] text-muted">
            Copy the incorrect spelling from History. Separate alternatives with commas.
          </span>
        </label>
      </div>
      <div className="flex flex-wrap gap-x-8 gap-y-3">
        <div className="flex flex-col gap-1.5 text-[13px]">
          <span className="font-medium">Kind</span>
          <Seg
            label="Kind"
            value={draft.kind}
            options={[
              ["term", "Term"],
              ["person", "Person"],
            ]}
            onChange={(kind) => setDraft({ ...draft, kind })}
          />
        </div>
        <div className="flex flex-col gap-1.5 text-[13px]">
          <span className="font-medium">Prompt priority</span>
          <Seg
            label="Prompt priority"
            value={draft.boost}
            options={[
              ["normal", "Normal"],
              ["strong", "Strong"],
            ]}
            onChange={(boost) => setDraft({ ...draft, boost })}
          />
          <span className="text-xs text-muted">Strong words get priority in prompts.</span>
        </div>
        <div className="flex min-w-0 grow flex-col gap-1.5 text-[13px]">
          <span className="font-medium">Apps</span>
          <AppPicker apps={draft.apps} suggestions={apps} onChange={(a) => setDraft({ ...draft, apps: a })} />
        </div>
      </div>
      <div className="flex justify-end gap-2">
        <button type="button" className={btn} onClick={onDone}>
          Cancel
        </button>
        <button type="submit" className={btnPrimary} disabled={busy || !draft.word.trim()}>
          {entry.id ? "Save" : "Add word"}
        </button>
      </div>
    </form>
  );
}

const COLUMNS = "grid grid-cols-[2fr_2fr_1fr_1.4fr_112px] gap-4";

function Row({ entry, replacementsOnly, onEdit, onError }: { entry: DictionaryEntry; replacementsOnly: boolean; onEdit: () => void; onError: (e: string) => void }) {
  return (
    <div className={`${COLUMNS} items-center border-b border-hair px-5 py-3.5 last:border-b-0`}>
      <span className="flex min-w-0 items-center gap-2 text-sm font-medium">
        <span className="truncate" title={entry.word}>
          {entry.word}
        </span>
        {entry.kind === "person" && (
          <span className="inline-flex h-5 shrink-0 items-center rounded-full bg-paper px-2 text-[11px] font-medium text-muted">
            Person
          </span>
        )}
      </span>
      <span className="truncate text-sm text-muted" title={entry.soundsLike.join(", ")}>
        {entry.soundsLike.length ? entry.soundsLike.join(", ") : replacementsOnly ? "Add incorrect spelling to correct" : "—"}
      </span>
      <span className="text-[13px]">{entry.boost === "strong" ? "Strong" : "Normal"}</span>
      <span className="truncate text-[13px] text-muted" title={entry.apps.join(", ")}>
        {entry.apps.length ? entry.apps.join(", ") : "All apps"}
      </span>
      <span className="flex justify-end gap-3">
        <button type="button" className="text-xs font-medium text-muted hover:text-ink" aria-label={`Edit ${entry.word}`} onClick={onEdit}>
          Edit
        </button>
        <button
          type="button"
          className="text-xs text-faint hover:text-rust"
          aria-label={`Remove ${entry.word}`}
          onClick={() => api.removeWord(entry.id).catch((e) => onError(errorText(e)))}
        >
          Remove
        </button>
      </span>
    </div>
  );
}

export function DictionaryPage({ s }: { s: Snapshot }) {
  const [history] = useHistory();
  const [filter, setFilter] = useState<Filter>("all");
  const [editing, setEditing] = useState<DictionaryEntry | null>(null);
  const [error, setError] = useState("");
  const words = s.settings.dictionary;
  const shown = words.filter((w) => matches(w, filter));
  const full = words.length >= MAX_WORDS;
  const apps = knownApps(history);
  const replacementsOnly = s.engine.active?.dictionary === "replacements";

  return (
    <main className="flex max-w-[1100px] flex-col gap-5 px-12 pt-9 pb-12">
      <div className="flex items-end justify-between gap-4">
        <div className="flex flex-col gap-1.5">
          <h1 className="m-0 font-serif text-[40px] font-normal">Dictionary</h1>
          <span className="text-[13px] leading-[1.45] text-muted">
            Names, jargon and spellings the engine should get right the first time.
          </span>
        </div>
        <button
          type="button"
          className={`${btnPrimary} h-9 rounded-[9px] px-3.5 text-sm`}
          disabled={full || editing?.id === ""}
          title={full ? `The dictionary holds up to ${MAX_WORDS} words` : undefined}
          onClick={() => setEditing(EMPTY)}
        >
          <Icon name="plus" />
          Add word
        </button>
      </div>

      <ErrorBanner error={error} onDismiss={() => setError("")} />

      <div className="rounded-[10px] border border-line bg-paper px-4 py-3 text-[13px] leading-[1.5] text-muted">
        {engineNote(s)}
      </div>

      {editing && (
        <Editor
          key={editing.id || "new"}
          entry={editing}
          apps={apps}
          replacementsOnly={replacementsOnly}
          onError={setError}
          onDone={() => setEditing(null)}
        />
      )}

      <div className="flex items-center justify-between gap-4">
        <div role="tablist" aria-label="Filter" className="flex gap-1">
          {FILTERS.map(([id, label]) => {
            const on = filter === id;
            const count = words.filter((w) => matches(w, id)).length;
            return (
              <button
                key={id}
                type="button"
                role="tab"
                aria-selected={on}
                onClick={() => setFilter(id)}
                className={
                  "h-[34px] rounded-full px-3.5 text-[13px] font-medium " +
                  (on ? "bg-ink text-white" : "text-muted hover:bg-sand")
                }
              >
                {label} · {count}
              </button>
            );
          })}
        </div>
        <div className="flex items-center gap-2.5">
          <span className="text-[13px] text-muted">
            {words.length} of {MAX_WORDS} words
          </span>
          <div
            role="meter"
            aria-label="Words used"
            aria-valuemin={0}
            aria-valuemax={MAX_WORDS}
            aria-valuenow={words.length}
            className="h-1.5 w-[120px] overflow-hidden rounded-full bg-line"
          >
            <div className="h-full bg-ink" style={{ width: `${(words.length / MAX_WORDS) * 100}%` }} />
          </div>
        </div>
      </div>

      <div className="overflow-hidden rounded-[14px] border border-line bg-white">
        <div className={`${COLUMNS} border-b border-sand px-5 py-3 text-xs font-semibold tracking-[.04em] text-faint uppercase`}>
          <span>Write as</span>
          <span>When I say</span>
          <span>Priority</span>
          <span>Apps</span>
          <span className="sr-only">Actions</span>
        </div>
        {shown.map((entry) => (
          <Row key={entry.id} entry={entry} replacementsOnly={replacementsOnly} onEdit={() => setEditing(entry)} onError={setError} />
        ))}
        {shown.length === 0 && (
          <div className="flex flex-col items-center gap-1.5 px-5 py-12 text-center">
            <Icon name="dictionary" size={22} className="shrink-0 text-faint" />
            <span className="text-sm font-medium">{words.length ? "Nothing here yet" : "No words yet"}</span>
            <span className="max-w-[420px] text-[13px] leading-[1.45] text-muted">
              {words.length
                ? "Words of this kind will show here."
                : "Add names, product terms and jargon, and what the engine writes instead when it mishears them."}
            </span>
          </div>
        )}
      </div>

    </main>
  );
}
