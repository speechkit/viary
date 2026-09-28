// The main window: the shared sidebar, and a page beside it.

import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { Icon, Mark, type IconName } from "../components/Icons";
import { useSnapshot, type Snapshot } from "../lib/ipc";
import { DictionaryPage } from "../pages/Dictionary";
import { EnginePage } from "../pages/Engine";
import { HistoryPage } from "../pages/History";
import { HomePage } from "../pages/Home";
import { PolishPage } from "../pages/Polish";
import { SettingsPage } from "../pages/Settings";

type Page = "home" | "history" | "dictionary" | "polish" | "engine" | "settings";

const NAV: { id: Page; label: string; icon: IconName }[] = [
  { id: "home", label: "Home", icon: "home" },
  { id: "history", label: "History", icon: "history" },
  { id: "dictionary", label: "Dictionary", icon: "dictionary" },
  { id: "polish", label: "Polish & tone", icon: "sparkle" },
  { id: "engine", label: "Voice engine", icon: "engine" },
  { id: "settings", label: "Settings", icon: "settings" },
];

function StatusCard({ s }: { s: Snapshot }) {
  const active = s.engine.active;
  let dot = "bg-teal";
  let title = "Ready · on-device";
  let text = "Audio never leaves this Mac with the current engine.";
  if (s.engine.loading) {
    dot = "bg-amber";
    title = "Loading engine…";
    text = "The next dictation uses it once it is ready.";
  } else if (!active) {
    dot = "bg-stone";
    title = "No voice engine";
    text = "Add a model folder or a cloud key under Voice engine.";
  } else if (!active.onDevice) {
    dot = "bg-blue";
    title = "Ready · cloud";
    text = `Audio is sent to ${active.kind} for recognition.`;
  }
  return (
    <div className="flex flex-col gap-1.5 rounded-xl border border-rule bg-card p-3">
      <div className="flex items-center gap-2 text-[13px] font-semibold">
        <span className={`size-2 rounded-full ${dot}`} />
        {title}
      </div>
      <span className="text-xs leading-[1.45] text-muted">{text}</span>
    </div>
  );
}

function Sidebar({ page, go, s }: { page: Page; go: (p: Page) => void; s: Snapshot }) {
  return (
    <nav
      aria-label="Main"
      className="flex h-full w-[232px] shrink-0 flex-col gap-1 border-r border-rule bg-sand px-3 pt-[46px] pb-4"
    >
      <div data-tauri-drag-region className="absolute top-0 left-0 h-[40px] w-[232px]" />
      <div className="flex items-center gap-2.5 px-2 pb-5">
        <span className="inline-flex size-[30px] items-center justify-center rounded-[7px] bg-ink">
          <Mark size={19} />
        </span>
        <span className="-mt-[3px] font-serif text-2xl leading-none font-medium tracking-[-0.02em]">viary</span>
      </div>
      {NAV.map((item) => {
        const current = item.id === page;
        return (
          <button
            key={item.id}
            type="button"
            aria-current={current ? "page" : undefined}
            onClick={() => go(item.id)}
            className={
              "flex h-[38px] items-center gap-2.5 rounded-[9px] px-2.5 text-left text-sm font-medium " +
              (current ? "bg-card text-ink" : "text-muted hover:bg-card/60")
            }
          >
            <Icon name={item.icon} size={18} strokeWidth={1.7} />
            <span className="grow">{item.label}</span>
          </button>
        );
      })}
      <div className="grow" />
      <StatusCard s={s} />
    </nav>
  );
}

export function MainWindow() {
  const [s] = useSnapshot();
  const [page, setPage] = useState<Page>("home");
  const [focusEntry, setFocusEntry] = useState<string | null>(null);

  useEffect(() => {
    const stop = listen<string>("navigate", ({ payload }) => {
      const [target, entry] = payload.split(":");
      setPage(target as Page);
      setFocusEntry(entry ?? null);
    });
    return () => {
      stop.then((f) => f());
    };
  }, []);

  if (!s) return null;
  return (
    <div className="relative flex h-full overflow-hidden bg-paper">
      <Sidebar page={page} go={setPage} s={s} />
      <div className="scroll relative min-w-0 grow">
        <div data-tauri-drag-region className="absolute inset-x-0 top-0 h-[28px]" />
        {page === "home" && <HomePage s={s} go={setPage} />}
        {page === "history" && <HistoryPage s={s} focus={focusEntry} />}
        {page === "dictionary" && <DictionaryPage s={s} />}
        {page === "polish" && <PolishPage s={s} />}
        {page === "engine" && <EnginePage s={s} />}
        {page === "settings" && <SettingsPage s={s} />}
      </div>
    </div>
  );
}
