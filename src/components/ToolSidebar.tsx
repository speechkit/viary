// The sidebar a tool shows in place of the main one (VoiceNotes and
// Transcripts artboards): back to Viary, the tool's title, search, and its
// main action. ⌘F opens a search row that covers only this tool.

import { useEffect, useRef, useState } from "react";
import { Icon } from "./Icons";
import { cmd, isMac, shortcut } from "../lib/platform";

const FIND = shortcut(["cmd"], "F");

export interface ToolSearch {
  open: boolean;
  query: string;
  setQuery: (query: string) => void;
  toggle: () => void;
  close: () => void;
  input: React.RefObject<HTMLInputElement | null>;
}

/** Search state for a tool: ⌘F opens and focuses it, Esc clears and closes it. */
export function useToolSearch(): ToolSearch {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (cmd(e) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        setOpen(true);
        // Already open: ⌘F puts the cursor back in the field.
        requestAnimationFrame(() => input.current?.select());
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const close = () => {
    setOpen(false);
    setQuery("");
  };
  return { open, query, setQuery, close, input, toggle: () => (open ? close() : setOpen(true)) };
}

const tbBtn =
  "inline-flex size-[30px] shrink-0 items-center justify-center rounded-[7px] text-ink hover:bg-ink/8";

export function ToolSidebar({
  label,
  title,
  onBack,
  search,
  searchPlaceholder,
  action,
  children,
}: {
  label: string;
  title: string;
  onBack: () => void;
  search: ToolSearch;
  searchPlaceholder: string;
  /** The tool's main action, as an icon button at the end of the title row. */
  action: { label: string; icon: React.ReactNode; onClick: () => void };
  children: React.ReactNode;
}) {
  return (
    <nav
      aria-label={label}
      className="relative flex h-full w-[300px] shrink-0 flex-col overflow-hidden border-r border-rule bg-sand"
    >
      <div data-tauri-drag-region className={`flex h-[52px] shrink-0 items-center gap-2 pr-3 ${isMac ? "pl-[88px]" : "pl-3"}`}>
        <button type="button" onClick={onBack} aria-label="Back to Viary" title="Back to Viary" className={tbBtn}>
          <Icon name="back" strokeWidth={2.2} />
        </button>
        <h1 data-tauri-drag-region className="m-0 grow truncate text-sm font-semibold">
          {title}
        </h1>
        <button
          type="button"
          onClick={() => {
            search.toggle();
            if (!search.open) requestAnimationFrame(() => search.input.current?.focus());
          }}
          aria-pressed={search.open}
          aria-label={`Search (${FIND})`}
          title={`Search (${FIND})`}
          className={tbBtn + (search.open ? " bg-ink/10" : "")}
        >
          <Icon name="search" size={18} />
        </button>
        <button type="button" onClick={action.onClick} aria-label={action.label} title={action.label} className={tbBtn}>
          {action.icon}
        </button>
      </div>
      {search.open && (
        <div className="px-3 pb-2">
          <label className="flex h-8 items-center gap-1.5 rounded-lg bg-white pr-1 pl-2.5 text-faint shadow-[0_0_0_2px_var(--color-blue)]">
            <Icon name="search" size={15} />
            <input
              ref={search.input}
              type="search"
              autoFocus
              placeholder={searchPlaceholder}
              aria-label={searchPlaceholder}
              value={search.query}
              onChange={(e) => search.setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.stopPropagation();
                  search.close();
                }
              }}
              className="w-0 grow border-0 bg-transparent text-[13px] text-ink outline-0 [&::-webkit-search-cancel-button]:hidden"
            />
            <button
              type="button"
              onClick={search.close}
              aria-label="Clear search"
              title="Clear (Esc)"
              className="inline-flex size-[22px] shrink-0 items-center justify-center rounded-full bg-sand text-muted"
            >
              <Icon name="close" size={12} strokeWidth={2.4} />
            </button>
          </label>
        </div>
      )}
      {children}
    </nav>
  );
}

/** The sidebar's message when a tool has nothing yet. */
export function SidebarEmpty({ icon, title, text }: { icon: React.ReactNode; title: string; text: string }) {
  return (
    <div className="flex grow flex-col items-center justify-center gap-2 px-8 pb-20 text-center">
      <span className="text-[#A39D90]">{icon}</span>
      <span className="text-[13px] font-semibold text-muted">{title}</span>
      <span className="text-xs leading-[1.45] text-faint">{text}</span>
    </div>
  );
}

/** A small rounded label, as under a note's title. */
export function MetaChip({ tone = "plain", children }: { tone?: "plain" | "device" | "cloud"; children: React.ReactNode }) {
  const colors = tone === "device" ? "bg-teal-wash text-teal-ink" : tone === "cloud" ? "bg-blue-wash text-blue" : "bg-sand text-muted";
  return <span className={`inline-flex h-[26px] items-center gap-1.5 rounded-full px-2.5 text-xs ${colors}`}>{children}</span>;
}
