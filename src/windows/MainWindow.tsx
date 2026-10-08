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
import { TranscriptsPage } from "../pages/Transcripts";
import { VoiceNotesPage } from "../pages/VoiceNotes";
import { isMac, THIS_COMPUTER } from "../lib/platform";

type MainPage = "home" | "history" | "dictionary" | "polish" | "engine" | "settings";
type Tool = "notes" | "transcripts";
type Page = MainPage | Tool;

const TOOLS: { id: Tool; label: string; icon: IconName }[] = [
  { id: "notes", label: "Voice Notes", icon: "notes" },
  { id: "transcripts", label: "Transcripts", icon: "transcript" },
];

export interface Intent {
  action: string | null;
  n: number;
}

const isTool = (page: Page): page is Tool => page === "notes" || page === "transcripts";

const NAV: { id: MainPage; label: string; icon: IconName }[] = [
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
  let text = `${active?.name}: audio never leaves ${THIS_COMPUTER}.`;
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
    text = `${active.name}: audio is sent to ${active.kind} for recognition.`;
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

function NavItem({ item, current, go }: { item: { id: Page; label: string; icon: IconName }; current: boolean; go: (p: Page) => void }) {
  return (
    <button
      type="button"
      aria-current={current ? "page" : undefined}
      onClick={() => go(item.id)}
      className={
        "flex h-[38px] shrink-0 items-center gap-2.5 rounded-[9px] px-2.5 text-left text-sm font-medium " +
        (current ? "bg-card text-ink" : "text-muted hover:bg-card/60")
      }
    >
      <Icon name={item.icon} size={18} strokeWidth={1.7} />
      <span className="grow">{item.label}</span>
    </button>
  );
}

function Sidebar({ page, go, s }: { page: Page; go: (p: Page) => void; s: Snapshot }) {
  return (
    <nav
      aria-label="Main"
      // macOS draws the traffic lights over the top of the sidebar.
      className={`flex h-full w-[232px] shrink-0 flex-col gap-1 border-r border-rule bg-sand px-3 pb-4 ${isMac ? "pt-[46px]" : "pt-5"}`}
    >
      <div data-tauri-drag-region className="absolute top-0 left-0 h-[40px] w-[232px]" />
      <div className="flex items-center gap-2.5 px-2 pb-5">
        <span className="inline-flex size-[30px] items-center justify-center rounded-[7px] bg-ink">
          <Mark size={19} />
        </span>
        <span className="-mt-[3px] font-serif text-2xl leading-none font-medium tracking-[-0.02em]">viary</span>
      </div>
      {NAV.map((item) => (
        <NavItem key={item.id} item={item} current={item.id === page} go={go} />
      ))}
      <div className="grow" />
      <span className="px-2.5 pb-1 text-[11px] font-semibold tracking-[0.06em] text-faint uppercase">Tools</span>
      {TOOLS.map((item) => (
        <NavItem key={item.id} item={item} current={item.id === page} go={go} />
      ))}
      <div className="h-3 shrink-0" />
      <StatusCard s={s} />
    </nav>
  );
}

export function MainWindow() {
  const [s] = useSnapshot();
  const [page, setPage] = useState<Page>("home");
  // Where "Back to Viary" returns from a tool.
  const [lastMain, setLastMain] = useState<MainPage>("home");
  const [focusEntry, setFocusEntry] = useState<string | null>(null);
  // What a tool was opened to do ("new", "choose"); `n` makes a repeat count.
  const [intent, setIntent] = useState<Intent>({ action: null, n: 0 });

  // Going somewhere by hand carries no request, so a tool opened again
  // later does not repeat the last one (such as opening the file picker).
  const go = (target: Page) => {
    if (!isTool(target)) setLastMain(target);
    setPage(target);
    setIntent((i) => ({ action: null, n: i.n }));
  };

  useEffect(() => {
    const stop = listen<string>("navigate", ({ payload }) => {
      const [target, entry] = payload.split(":");
      if (!isTool(target as Page)) setLastMain(target as MainPage);
      setPage(target as Page);
      setFocusEntry(entry ?? null);
      setIntent((i) => ({ action: entry ?? null, n: i.n + 1 }));
    });
    return () => {
      stop.then((f) => f());
    };
  }, []);

  if (!s) return null;
  if (isTool(page)) {
    // A tool's own sidebar replaces the main one.
    const back = () => setPage(lastMain);
    return (
      <div className="relative flex h-full overflow-hidden bg-paper">
        {page === "notes" && <VoiceNotesPage s={s} onBack={back} intent={intent} />}
        {page === "transcripts" && <TranscriptsPage s={s} onBack={back} intent={intent} />}
      </div>
    );
  }
  return (
    <div className="relative flex h-full overflow-hidden bg-paper">
      <Sidebar page={page} go={go} s={s} />
      <div className="scroll relative min-w-0 grow">
        <div data-tauri-drag-region className="absolute inset-x-0 top-0 h-[28px]" />
        {page === "home" && <HomePage s={s} go={go} />}
        {page === "history" && <HistoryPage s={s} focus={focusEntry} />}
        {page === "dictionary" && <DictionaryPage s={s} />}
        {page === "polish" && <PolishPage s={s} />}
        {page === "engine" && <EnginePage s={s} />}
        {page === "settings" && <SettingsPage s={s} />}
      </div>
    </div>
  );
}
