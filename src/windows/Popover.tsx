// The menu bar popover (MenuBar artboard): status, the active engine,
// language, polish and translation, microphone, recent dictations.

import { LogicalSize } from "@tauri-apps/api/dpi";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { useEffect, useRef } from "react";
import { Switch } from "../components/Controls";
import { AppTile, Icon, Kbd } from "../components/Icons";
import { api, engineName, useHistory, useSnapshot, type Language, type Snapshot } from "../lib/ipc";

const LANGUAGES: [Language, string][] = [
  ["auto", "Auto"],
  ["en", "English"],
  ["zh", "中文"],
];

/** The status badge beside the app name: a word, its color, and a hint. */
type Status = { label: string; tone: "ok" | "live" | "busy" | "attention" | "off"; hint: string };

const TONES: Record<Status["tone"], { dot: string; badge: string }> = {
  ok: { dot: "bg-teal", badge: "bg-teal-wash text-teal-ink" },
  live: { dot: "bg-live", badge: "bg-rust-wash text-rust" },
  busy: { dot: "bg-blue", badge: "bg-blue-wash text-blue" },
  attention: { dot: "bg-amber", badge: "bg-sand text-body" },
  off: { dot: "bg-stone", badge: "bg-sand text-muted" },
};

function status(s: Snapshot): Status {
  const hold = `Hold ${s.hotkeyName} to dictate`;
  switch (s.pill.kind) {
    case "listening":
      return { label: "Listening", tone: "live", hint: "Release to insert" };
    case "handsFree":
      return { label: "Listening", tone: "live", hint: `Hands-free · tap ${s.hotkeyName} to insert` };
    case "transcribing":
      return { label: "Transcribing", tone: "busy", hint: hold };
    case "polishing":
      return { label: "Polishing", tone: "busy", hint: "Skip from the pill to insert it as recognized" };
    case "failed":
      return { label: "Failed", tone: "attention", hint: "The audio is kept: retry from the pill" };
  }
  if (s.engine.loading) return { label: "Loading", tone: "busy", hint: "The voice engine is loading" };
  if (!s.engine.active) return { label: "No engine", tone: "off", hint: "Choose a voice engine to transcribe" };
  if (!s.hotkeyActive) return { label: "Almost ready", tone: "attention", hint: `Allow Input Monitoring to use ${s.hotkeyName}` };
  return { label: "Ready", tone: "ok", hint: hold };
}

function EngineBanner({ s }: { s: Snapshot }) {
  const { active, loading } = s.engine;
  const open = () => api.openMain("engine");
  if (loading) {
    return (
      <button type="button" onClick={open} className="flex w-full items-center gap-2 rounded-[10px] bg-sand px-3 py-2.5 text-left text-[13px] text-muted">
        <Icon name="refresh" />
        Loading {engineName(s, loading)}…
      </button>
    );
  }
  if (!active) {
    return (
      <button type="button" onClick={open} className="flex w-full items-center gap-2 rounded-[10px] bg-sand px-3 py-2.5 text-left text-[13px] text-ink">
        <Icon name="engine" />
        No voice engine · Choose one
      </button>
    );
  }
  return (
    <button
      type="button"
      onClick={open}
      title="Change the voice engine"
      className={
        "flex w-full items-center gap-2 rounded-[10px] px-3 py-2.5 text-left text-[13px] " +
        (active.onDevice ? "bg-teal-wash text-teal-ink" : "bg-blue-wash text-blue")
      }
    >
      <Icon name={active.onDevice ? "laptop" : "cloud"} />
      <span className="truncate">
        {active.onDevice ? `On-device · ${active.name}` : `Cloud · ${active.kind} · ${active.name}`}
      </span>
    </button>
  );
}

export function Popover() {
  const [s] = useSnapshot();
  const [history] = useHistory();
  const card = useRef<HTMLDivElement>(null);

  // Fit the window to the card, so the empty space below takes no clicks;
  // on a screen too short for it, the card scrolls instead.
  useEffect(() => {
    if (!card.current) return;
    const popover = getCurrentWebviewWindow();
    let height = 0;
    const fit = () => {
      if (!height) return;
      // The popover opens below the menu bar; keep its bottom on screen.
      const room = window.screen.availHeight - 40;
      popover.setSize(new LogicalSize(360, Math.min(height + 24, room))).catch(console.error);
    };
    const observer = new ResizeObserver(([entry]) => {
      height = Math.ceil(entry.contentRect.height);
      fit();
    });
    observer.observe(card.current);
    // The screen may have changed since the popover last opened.
    window.addEventListener("focus", fit);
    return () => {
      observer.disconnect();
      window.removeEventListener("focus", fit);
    };
  }, [s !== null]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") api.hidePopover();
      if (e.metaKey && e.key === ",") api.openMain("home");
      if (e.metaKey && e.key.toLowerCase() === "q") api.quit();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (!s) return null;
  const state = status(s);
  const canPickLanguage = s.engine.active?.languageOverride ?? false;
  const recent = (history ?? []).filter((h) => h.text).slice(0, 2);
  const polish = s.settings.polish;

  return (
    <div className="p-3">
      <div className="scroll max-h-[calc(100vh-24px)] rounded-[14px] bg-card shadow-[0_8px_24px_rgba(28,27,24,0.22),0_0_0_1px_rgba(28,27,24,0.08)]">
        <div ref={card} className="flex flex-col">
          <div className="flex items-center gap-3 px-3.5 pt-4 pb-3">
            <AppTile />
            <div className="flex grow items-center gap-2">
              <span className="text-[15px] font-semibold">Viary</span>
              <span
                title={`Hold ${s.hotkeyName} to dictate`}
                aria-label={`Hold ${s.hotkeyName} to dictate`}
                className="inline-flex"
              >
                <span aria-hidden="true">
                  <Kbd>{s.hotkeyName}</Kbd>
                </span>
              </span>
            </div>
            <span
              role="status"
              title={state.hint}
              aria-label={`${state.label}. ${state.hint}`}
              className={`inline-flex h-6 shrink-0 items-center gap-1.5 rounded-full px-2.5 text-xs font-medium ${TONES[state.tone].badge}`}
            >
              <span aria-hidden="true" className={`size-1.5 rounded-full ${TONES[state.tone].dot}`} />
              {state.label}
            </span>
          </div>
          {!s.hotkeyActive && (
            <div className="mx-3.5 mb-3">
              <button
                type="button"
                onClick={() => api.requestPermission("inputMonitoring")}
                className="h-8 w-full rounded-lg border border-edge bg-white text-[13px] font-medium"
              >
                Allow Input Monitoring…
              </button>
            </div>
          )}
          <div className="mx-3.5 mb-3">
            <EngineBanner s={s} />
          </div>
          <div className="flex flex-col gap-2 px-3.5 pb-3">
            <span className="text-xs text-faint">Language</span>
            <div
              role="radiogroup"
              aria-label="Language"
              className={"flex gap-0.5 rounded-[9px] bg-sand p-[3px] " + (canPickLanguage ? "" : "opacity-50")}
              title={canPickLanguage ? undefined : "This engine detects the language itself"}
            >
              {LANGUAGES.map(([value, label]) => {
                const on = s.settings.language === value;
                return (
                  <button
                    key={value}
                    type="button"
                    role="radio"
                    aria-checked={on}
                    disabled={!canPickLanguage}
                    onClick={() => api.setPreferences({ language: value })}
                    className={
                      "h-[30px] flex-1 rounded-[7px] border-0 text-[13px] font-medium " +
                      (on ? "bg-white text-ink shadow-[0_1px_2px_rgba(0,0,0,.12)]" : "bg-transparent text-muted")
                    }
                  >
                    {label}
                  </button>
                );
              })}
            </div>
          </div>
          <div className="border-t border-[#E8E3D8]">
            <div className="flex min-h-11 items-center gap-2.5 px-3.5 text-sm">
              <Icon name="sparkle" />
              <button type="button" className="grow text-left" onClick={() => api.openMain("polish")}>
                Polish transcripts
              </button>
              <Switch
                small
                label="Polish transcripts"
                on={polish.enabled}
                onChange={(enabled) => api.updatePolish({ enabled }).catch(console.error)}
              />
            </div>
            <div className="flex min-h-11 items-center gap-2.5 px-3.5 text-sm">
              <Icon name="translate" />
              <span className="grow">Translate to {polish.translateTo}</span>
              <Switch
                small
                label={`Translate to ${polish.translateTo}`}
                on={polish.enabled && polish.translate}
                disabled={!polish.enabled}
                onChange={(translate) => api.updatePolish({ translate }).catch(console.error)}
              />
            </div>
            <button
              type="button"
              onClick={() => api.openMain("engine")}
              className="flex min-h-11 w-full items-center gap-2.5 px-3.5 text-left text-sm"
            >
              <Icon name="mic" />
              <span className="grow truncate">{s.settings.microphone ?? "Default microphone"}</span>
              <span className="text-xs text-faint">Change</span>
            </button>
          </div>
          {recent.length > 0 && (
            <div className="border-t border-[#E8E3D8] py-2">
              <span className="block px-3.5 py-1 text-xs text-faint">Recent</span>
              {recent.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  onClick={() => api.openMain(`history:${item.id}`)}
                  className="flex min-h-10 w-full items-center justify-between gap-3 px-3.5 text-left text-sm"
                >
                  <span className="truncate">{item.text}</span>
                  <span className="shrink-0 text-xs text-faint">{item.app || "—"}</span>
                </button>
              ))}
            </div>
          )}
          <div className="border-t border-[#E8E3D8] py-1.5">
            <button type="button" onClick={() => api.newVoiceNote().catch(console.error)} className="flex min-h-10 w-full items-center justify-between px-3.5 text-sm">
              <span className="flex items-center gap-2.5">
                <Icon name="notes" />
                New voice note
              </span>
              <Kbd>⌥⌘N</Kbd>
            </button>
            <button
              type="button"
              onClick={() => api.openMain("transcripts:choose")}
              className="flex min-h-10 w-full items-center justify-between px-3.5 text-sm"
            >
              <span className="flex items-center gap-2.5">
                <Icon name="transcript" />
                Transcribe a file…
              </span>
              <span className="text-xs text-faint">or drop on icon</span>
            </button>
          </div>
          <div className="border-t border-[#E8E3D8] py-1.5">
            <button type="button" onClick={() => api.openMain("home")} className="flex min-h-10 w-full items-center justify-between px-3.5 text-sm">
              Open Viary <Kbd>⌘,</Kbd>
            </button>
            <button type="button" onClick={() => api.quit()} className="flex min-h-10 w-full items-center justify-between px-3.5 text-sm">
              Quit <Kbd>⌘Q</Kbd>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
