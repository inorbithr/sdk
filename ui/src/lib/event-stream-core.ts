// The live event stream's data (Sensors' control room and Workload's share it): one line per event
// with when, its tone (a health or a state), its text and the group it came from (a source, an agent
// or a job group), filterable by group and tone. Pure.

export type StreamEvent<T extends string> = {
  id: string;
  at: number;
  tone: T;
  text: string;
  /** Where it came from: a source id, an agent's node id, a job group. */
  group: string;
  /** Said in bold: a change of health or state, not a plain signal. */
  strong?: boolean;
};

/** The events of one group (null for every group) in any of the tones (none for every tone). */
export function filterStream<T extends string>(events: readonly StreamEvent<T>[], group: string | null, tones: readonly T[]): StreamEvent<T>[] {
  return events.filter((e) => (!group || e.group === group) && (!tones.length || tones.includes(e.tone)));
}
