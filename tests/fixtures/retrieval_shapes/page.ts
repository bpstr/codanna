/** Merge an incoming page with the current projection. */
export function mergePage(current: number[], incoming: number[]): number[] {
  return [...new Set([...current, ...incoming])];
}
