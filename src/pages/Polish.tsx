// Polish & tone (Polish artboard): a language model turns what you said into
// what you meant, after recognition and before insertion, in a tone set
// per app.

import { useEffect, useState } from "react";
import { btn, btnDanger, btnPrimary, ErrorBanner, Field, input, Select, Switch, useDraft } from "../components/Controls";
import { Icon } from "../components/Icons";
import {
  alsoIn,
  api,
  errorText,
  knownApps,
  toneIn,
  useHistory,
  type PolishProvider,
  type PolishSettings,
  type Snapshot,
  type Tone,
} from "../lib/ipc";
import { KEY_STORE, THIS_COMPUTER } from "../lib/platform";

const TONES: { value: Tone; label: string }[] = [
  { value: "formal", label: "Formal" },
  { value: "casual", label: "Casual" },
  { value: "asSpoken", label: "As spoken" },
  { value: "literal", label: "Literal · no polish" },
];

const LANGUAGES = [
  { value: "English", label: "English" },
  { value: "Chinese", label: "中文" },
  { value: "Japanese", label: "日本語" },
  { value: "Korean", label: "한국어" },
  { value: "Spanish", label: "Español" },
  { value: "French", label: "Français" },
  { value: "German", label: "Deutsch" },
];

const PROVIDERS: { value: PolishProvider; label: string }[] = [
  { value: "local", label: "Local server" },
  { value: "openAi", label: "OpenAI" },
  { value: "dashScope", label: "DashScope" },
  { value: "custom", label: "Custom" },
];

const SAMPLE =
  "um so can you uh rerun the nightly with, like, two test threads, no wait, three, because models aborted again";

/** White text passes 4.5:1 on each. */
const TILE_COLORS = ["#2F6FD6", "#5B2D6E", "#8A6A12", "#1F6FB2", "#0F766E", "#A33B12", "#1C1B18"];

function AppTile({ app }: { app: string }) {
  let hash = 0;
  for (const c of app) hash = (hash * 31 + c.charCodeAt(0)) >>> 0;
  return (
    <span
      aria-hidden="true"
      className="flex size-7 shrink-0 items-center justify-center rounded-[7px] text-xs font-semibold text-white"
      style={{ background: TILE_COLORS[hash % TILE_COLORS.length] }}
    >
      {[...app][0]?.toUpperCase()}
    </span>
  );
}

function Rule({
  name,
  desc,
  on,
  disabled,
  onChange,
  children,
}: {
  name: string;
  desc: string;
  on: boolean;
  disabled: boolean;
  onChange: (on: boolean) => void;
  children?: React.ReactNode;
}) {
  return (
    <div className={"flex items-center gap-4 border-b border-hair px-5 py-3.5 last:border-b-0 " + (disabled ? "opacity-50" : "")}>
      <div className="flex min-w-0 grow flex-col gap-0.5">
        <span className="text-sm font-medium">{name}</span>
        <span className="text-[13px] leading-[1.45] text-muted">{desc}</span>
      </div>
      {children}
      <Switch label={name} on={on} disabled={disabled} onChange={onChange} />
    </div>
  );
}

/** Describe the actual destination, including remote URLs in Local mode. */
function destination(baseUrl: string): string {
  try {
    const url = new URL(baseUrl);
    if (!["http:", "https:"].includes(url.protocol)) return "Enter an HTTP or HTTPS base URL.";
    const host = url.hostname.toLowerCase();
    const local = host === "localhost" || host === "localhost." || host === "[::1]" || /^127\.\d+\.\d+\.\d+$/.test(host);
    return local
      ? `Transcripts are sent to a local server on ${THIS_COMPUTER}.`
      : `Transcripts are sent to ${url.host}. Audio is not sent for polishing.`;
  } catch {
    return "Enter a valid server base URL.";
  }
}

/** Where the language model runs, and which one. */
function ModelCard({ s, onError }: { s: Snapshot; onError: (e: string) => void }) {
  const saved = s.settings.polish;
  const [draft, setDraft, dirty] = useDraft({ provider: saved.provider, baseUrl: saved.baseUrl, model: saved.model });
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState<"save" | "test" | "remove" | null>(null);
  const [connection, setConnection] = useState<{ text: string; error: boolean } | null>(null);
  const custom = draft.provider === "custom";
  const editableUrl = custom || draft.provider === "local";
  const keyDirty = custom && !!key.trim();
  const configured = !!draft.model.trim() && (!editableUrl || !!draft.baseUrl.trim());
  const fingerprint = JSON.stringify([draft, key, s.keys, s.settings.openai.baseUrl, s.settings.dashscope.region]);
  useEffect(() => setConnection(null), [fingerprint]);

  let note: { text: string; warn: boolean };
  if (editableUrl) {
    note = {
      text: `${custom ? "Connect to any OpenAI-compatible chat API. " : ""}${destination(draft.baseUrl)}`,
      warn: false,
    };
  } else if (draft.provider === "openAi") {
    note = s.keys.openAi
      ? { text: `Uses your OpenAI key and base URL from Voice engine. ${destination(s.settings.openai.baseUrl)}`, warn: false }
      : { text: "Add an OpenAI API key under Voice engine first.", warn: true };
  } else {
    note = s.keys.dashScope
      ? { text: "Uses your DashScope key and region from Voice engine. Transcripts are sent to DashScope.", warn: false }
      : { text: "Add a DashScope API key under Voice engine first.", warn: true };
  }

  const save = async () => {
    onError("");
    setBusy("save");
    try {
      if (keyDirty) {
        await api.saveApiKey("customPolish", key.trim());
        setKey("");
      }
      await api.updatePolish({ ...draft, baseUrl: draft.baseUrl.trim(), model: draft.model.trim() });
    } catch (e) {
      onError(errorText(e));
    } finally {
      setBusy(null);
    }
  };
  const test = async () => {
    setConnection(null);
    setBusy("test");
    try {
      await api.testPolishConnection(draft, keyDirty ? key.trim() : null);
      setConnection({ text: "Connected. The model returned a response.", error: false });
    } catch (e) {
      setConnection({ text: errorText(e), error: true });
    } finally {
      setBusy(null);
    }
  };
  const removeKey = async () => {
    onError("");
    setBusy("remove");
    try {
      await api.deleteApiKey("customPolish");
      setKey("");
    } catch (e) {
      onError(errorText(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="rounded-[14px] border border-line bg-white px-5 py-4">
      <fieldset disabled={!!busy} className="m-0 flex min-w-0 flex-col gap-3 border-0 p-0">
        <div className="flex flex-wrap items-start gap-3">
          <div className="flex min-w-[160px] grow basis-[180px] flex-col gap-0.5">
            <span className="text-sm font-medium">Polish model</span>
            <span className={`text-[13px] leading-[1.45] ${note.warn ? "text-rust" : "text-muted"}`}>{note.text}</span>
          </div>
          <Select
            label="Polish model provider"
            value={draft.provider}
            options={PROVIDERS}
            disabled={!!busy}
            onChange={(provider) => {
              setDraft({ ...draft, provider });
              setKey("");
            }}
          />
        </div>
        {editableUrl && (
          <>
            <Field label="Base URL">
              <input
                className={`${input} min-w-0 w-full`}
                spellCheck={false}
                placeholder="https://your-server.example/v1"
                value={draft.baseUrl}
                onChange={(e) => setDraft({ ...draft, baseUrl: e.target.value })}
              />
            </Field>
            <span className="text-xs leading-[1.45] text-faint">Enter the API base URL, including /v1 if required. For Ollama: http://localhost:11434/v1.</span>
          </>
        )}
        {custom && (
          <>
            <Field label="API Key">
              <input
                type="password"
                className={`${input} min-w-0 w-full`}
                autoComplete="off"
                spellCheck={false}
                placeholder={s.keys.customPolish ? "Enter a new key to replace the stored key" : "Optional for servers without authentication"}
                value={key}
                onChange={(e) => setKey(e.target.value)}
              />
            </Field>
            <div className="flex flex-wrap items-center gap-2 text-xs text-muted">
              <span className="grow">{s.keys.customPolish ? `API key stored in ${KEY_STORE}. Leave blank to keep it.` : `Saved keys are stored in ${KEY_STORE}.`}</span>
              {s.keys.customPolish && (
                <button type="button" className={btnDanger} onClick={removeKey}>Remove key</button>
              )}
            </div>
          </>
        )}
        <Field label="Model">
          <input
            className={`${input} min-w-0 w-full`}
            spellCheck={false}
            placeholder="The model ID, as the server lists it"
            value={draft.model}
            onChange={(e) => setDraft({ ...draft, model: e.target.value })}
          />
        </Field>
        {connection && (
          <span role={connection.error ? "alert" : "status"} className={`break-words text-[13px] ${connection.error ? "text-rust" : "text-teal-ink"}`}>
            {connection.text}
          </span>
        )}
        <div className="flex flex-wrap justify-end gap-2">
          <button type="button" className={btn} disabled={!configured || note.warn} onClick={test}>
            {busy === "test" ? "Testing…" : "Test connection"}
          </button>
          <button type="button" className={btnPrimary} disabled={!dirty && !keyDirty} onClick={save}>
            {busy === "save" ? "Saving…" : "Save"}
          </button>
        </div>
      </fieldset>
    </div>
  );
}

function AddApp({ options, onAdd, onCancel }: { options: string[]; onAdd: (app: string) => void; onCancel: () => void }) {
  const [typed, setTyped] = useState("");
  return (
    <form
      className="flex flex-col gap-2.5 border-b border-hair bg-card px-5 py-3.5"
      onSubmit={(e) => {
        e.preventDefault();
        if (typed.trim()) onAdd(typed.trim());
      }}
      onKeyDown={(e) => e.key === "Escape" && onCancel()}
    >
      {options.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {options.map((app) => (
            <button
              key={app}
              type="button"
              onClick={() => onAdd(app)}
              className="inline-flex h-7 items-center gap-1.5 rounded-full border border-edge bg-white px-2.5 text-[13px] hover:border-stone"
            >
              <Icon name="plus" size={12} />
              {app}
            </button>
          ))}
        </div>
      )}
      <div className="flex gap-2">
        <input
          className={`${input} grow`}
          autoFocus
          spellCheck={false}
          aria-label="App name"
          placeholder={options.length ? "Or type an app’s name" : "The app’s name, as History shows it"}
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
        />
        <button type="submit" className={btnPrimary} disabled={!typed.trim()}>
          Add
        </button>
        <button type="button" className={btn} onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}

function TonesCard({ s, onError }: { s: Snapshot; onError: (e: string) => void }) {
  const [history] = useHistory();
  const [adding, setAdding] = useState(false);
  const polish = s.settings.polish;
  const listed = polish.tones.map((t) => t.app);
  const seen = knownApps(history);
  const suggestions = seen.filter((app) => !listed.includes(app));
  const setTone = (app: string, tone: Tone | null) => api.setAppTone(app, tone).catch((e) => onError(errorText(e)));
  const dim = !polish.enabled ? "opacity-50" : "";
  return (
    <div className="overflow-hidden rounded-[14px] border border-line bg-white">
      <div className="flex items-center justify-between border-b border-sand px-5 py-3.5">
        <h2 className="m-0 text-[15px] font-semibold">Tone per app</h2>
        {!adding && (
          <button type="button" className="text-[13px] font-medium text-blue" onClick={() => setAdding(true)}>
            Add app
          </button>
        )}
      </div>
      {adding && (
        <AddApp
          options={suggestions.slice(0, 8)}
          onCancel={() => setAdding(false)}
          onAdd={(app) => {
            setAdding(false);
            if (!listed.some((a) => a.toLowerCase() === app.toLowerCase())) setTone(app, polish.defaultTone);
          }}
        />
      )}
      <div className={dim}>
        {polish.tones.map(({ app, tone }) => (
          <div key={app} className="group flex items-center gap-3 border-b border-hair px-5 py-2.5">
            <AppTile app={app} />
            <span className="flex min-w-0 grow flex-col">
              <span className="truncate text-sm">{app}</span>
              {alsoIn(app, listed, seen).length > 0 && (
                <span className="truncate text-xs text-faint" title={alsoIn(app, listed, seen).join(", ")}>
                  Also in {alsoIn(app, listed, seen).join(", ")}
                </span>
              )}
            </span>
            <Select label={`${app} tone`} value={tone} options={TONES} onChange={(t) => setTone(app, t)} />
            <button
              type="button"
              aria-label={`Remove ${app}`}
              className="text-faint opacity-0 group-focus-within:opacity-100 group-hover:opacity-100 hover:text-rust"
              onClick={() => setTone(app, null)}
            >
              <Icon name="close" size={14} />
            </button>
          </div>
        ))}
        <div className="flex items-center gap-3 px-5 py-2.5">
          <span aria-hidden="true" className="flex size-7 shrink-0 items-center justify-center rounded-[7px] border border-dashed border-stone text-faint">
            <Icon name="plus" size={12} />
          </span>
          <span className="min-w-0 grow text-sm text-muted">Other apps</span>
          <Select
            label="Tone in other apps"
            value={polish.defaultTone}
            options={TONES}
            onChange={(defaultTone) => api.updatePolish({ defaultTone }).catch((e) => onError(errorText(e)))}
          />
          <span className="w-3.5" />
        </div>
      </div>
      {!polish.appTone && polish.enabled && (
        <div className="border-t border-hair px-5 py-2.5 text-xs leading-[1.45] text-muted">
          Matching the app’s tone is off: only “Literal” apps are treated differently.
        </div>
      )}
    </div>
  );
}

/** Runs the polish on a sample, with the tone of the app picked. */
function PreviewCard({ s }: { s: Snapshot }) {
  const polish = s.settings.polish;
  const [app, setApp] = useState(polish.tones[0]?.app ?? "");
  const [text, setText] = useState(SAMPLE);
  const [result, setResult] = useState<{ text: string; error: boolean } | null>(null);
  const [busy, setBusy] = useState(false);
  const tone = TONES.find((t) => t.value === toneIn(polish, app))!;
  const ready = !!polish.model;
  const run = async () => {
    setBusy(true);
    setResult(null);
    try {
      setResult({ text: await api.polishPreview(app, text), error: false });
    } catch (e) {
      setResult({ text: errorText(e), error: true });
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="flex flex-col gap-3 rounded-[14px] border border-line bg-white px-5 py-[18px]">
      <div className="flex items-center justify-between gap-3">
        <span className="text-xs font-semibold tracking-[.04em] text-faint uppercase">
          Preview · {app || "Other apps"}, {tone.label.toLowerCase()}
        </span>
        <Select
          label="Preview app"
          value={app}
          options={[...polish.tones.map((t) => ({ value: t.app, label: t.app })), { value: "", label: "Other apps" }]}
          onChange={(a) => {
            setApp(a);
            setResult(null);
          }}
        />
      </div>
      <textarea
        aria-label="What you said"
        rows={3}
        spellCheck={false}
        className="resize-none rounded-lg border border-edge bg-white px-3 py-2 text-sm leading-[1.6] text-muted outline-none focus:border-ink"
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          setResult(null);
        }}
      />
      <div className="h-px bg-sand" />
      {result ? (
        <p role="status" className={`m-0 text-[15px] leading-[1.6] ${result.error ? "text-rust" : "selectable"}`}>
          {result.text}
        </p>
      ) : (
        <p className="m-0 text-[13px] leading-[1.6] text-faint">
          {!ready
            ? "Save a polish model to try it."
            : tone.value === "literal"
              ? "This app is set to Literal: its text is inserted as recognized."
              : "Run it to see what would be inserted."}
        </p>
      )}
      <div className="flex justify-end">
        <button
          type="button"
          className={btnPrimary}
          disabled={!ready || busy || !text.trim() || tone.value === "literal"}
          onClick={run}
        >
          {busy ? "Polishing…" : "Try it"}
        </button>
      </div>
    </div>
  );
}

export function PolishPage({ s }: { s: Snapshot }) {
  const [error, setError] = useState("");
  const polish = s.settings.polish;
  const save = (next: Partial<Omit<PolishSettings, "tones">>) => api.updatePolish(next).catch((e) => setError(errorText(e)));
  const off = !polish.enabled;
  const needsModel = polish.enabled && !polish.model;

  return (
    <main className="grid max-w-[1100px] grid-cols-1 items-start gap-x-7 gap-y-[18px] px-12 pt-9 pb-12 min-[1100px]:grid-cols-2">
      <div className="flex flex-col gap-[18px] min-[1100px]:col-span-2">
        <div className="flex flex-col gap-1.5">
          <h1 className="m-0 font-serif text-[40px] font-normal">Polish &amp; tone</h1>
          <span className="text-[13px] leading-[1.45] text-muted">
            Turns what you said into what you meant, after recognition and before insertion.
          </span>
        </div>
        <ErrorBanner error={error} onDismiss={() => setError("")} />
      </div>

      <div className="flex flex-col gap-[18px]">
        <div className="overflow-hidden rounded-[14px] border border-line bg-white">
          <div className="flex items-center gap-4 border-b border-hair bg-card px-5 py-3.5">
            <div className="flex min-w-0 grow flex-col gap-0.5">
              <span className="text-[15px] font-semibold">Polish transcripts</span>
              <span className={`text-[13px] leading-[1.45] ${needsModel ? "text-rust" : "text-muted"}`}>
                {off
                  ? "Off: punctuation only, exactly your words."
                  : needsModel
                    ? "Choose a polish model below. Until then, text is inserted as recognized."
                    : "On: the model rewrites each dictation before it is inserted."}
              </span>
            </div>
            <Switch label="Polish transcripts" on={polish.enabled} onChange={(enabled) => save({ enabled })} />
          </div>
          <Rule
            name="Remove filler words"
            desc="um, uh, like, you know, 那个, 就是"
            on={polish.removeFillers}
            disabled={off}
            onChange={(removeFillers) => save({ removeFillers })}
          />
          <Rule
            name="Apply self-corrections"
            desc="“Tuesday, no, Wednesday” becomes “Wednesday”"
            on={polish.selfCorrections}
            disabled={off}
            onChange={(selfCorrections) => save({ selfCorrections })}
          />
          <Rule
            name="Format lists and steps"
            desc="“first… second…” becomes a numbered list"
            on={polish.formatLists}
            disabled={off}
            onChange={(formatLists) => save({ formatLists })}
          />
          <Rule
            name="Match the app’s tone"
            desc="Uses the tone set for each app"
            on={polish.appTone}
            disabled={off}
            onChange={(appTone) => save({ appTone })}
          />
          <Rule
            name="Translate"
            desc={`Speak any language, insert ${LANGUAGES.find((l) => l.value === polish.translateTo)?.label ?? polish.translateTo}`}
            on={polish.translate}
            disabled={off}
            onChange={(translate) => save({ translate })}
          >
            {polish.translate && (
              <Select
                label="Translate to"
                value={polish.translateTo}
                options={LANGUAGES}
                disabled={off}
                onChange={(translateTo) => save({ translateTo })}
              />
            )}
          </Rule>
        </div>

        <ModelCard s={s} onError={setError} />
      </div>

      <div className="flex flex-col gap-[18px]">
        <TonesCard s={s} onError={setError} />
        <PreviewCard s={s} />
      </div>
    </main>
  );
}
