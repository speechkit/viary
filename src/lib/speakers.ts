// Speaker colors, in the order speakers are found. Ink when unknown.

const COLORS = ["#2F46C8", "#C2410C", "#0F766E", "#7C3AED", "#B45309", "#BE185D"];

export function speakerColor(index: number | null): string {
  return index === null ? "#1C1B18" : COLORS[index % COLORS.length];
}
