// Helpers Voice Notes and Transcripts share for showing passages and dates.

import type { Passage } from "./ipc";

/** The passage playing at `pos`: the last one that has started, or -1. */
export function currentIndex(passages: Passage[], pos: number): number {
  let current = -1;
  passages.forEach((p, i) => {
    if (pos >= p.startMs) current = i;
  });
  return current;
}

const han = /\p{Script=Han}|\p{Script=Hiragana}|\p{Script=Katakana}|\p{Script=Hangul}/u;

/** Words of Latin text take a space between them; Chinese and Japanese do not. */
export function spaced(before: string, after: string): boolean {
  return !han.test(before.slice(-1)) && !han.test(after.charAt(0)) && !/^[.,!?;:]/.test(after);
}

/** Where `query` occurs in `text`, ignoring case: [start, end) ranges in
 *  `text` itself, so they stay right when lowercasing changes lengths. */
export function matchRanges(text: string, query: string): [number, number][] {
  const q = query.trim();
  if (!q) return [];
  const pattern = new RegExp(q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "giu");
  return [...text.matchAll(pattern)].map((m) => [m.index, m.index + m[0].length]);
}

/** Calendar days between `ms` and today: 0 today, 1 yesterday. */
export function daysAgo(ms: number): number {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  return Math.floor((today.getTime() - new Date(ms).setHours(0, 0, 0, 0)) / 86_400_000);
}

/** A cue time as subtitles write it: 00:01:02,500 (SRT) or 00:01:02.500 (VTT),
 *  the same as the backend's `subtitles::cue_time`. */
export function cueTime(ms: number, format: "srt" | "vtt"): string {
  const pad = (n: number, width = 2) => String(Math.floor(n)).padStart(width, "0");
  return `${pad(ms / 3_600_000)}:${pad((ms % 3_600_000) / 60_000)}:${pad((ms % 60_000) / 1000)}${format === "srt" ? "," : "."}${pad(ms % 1000, 3)}`;
}
