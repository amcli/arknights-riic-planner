// Typed client for the ak-api endpoints. Types mirror the serde output of
// the Rust domain structs; keep them in sync by hand until an OpenAPI/TS
// generator is wired in. Every error body is `{ "error": message }`.

export type RoomType =
  | "CONTROL"
  | "POWER"
  | "MANUFACTURE"
  | "TRADING"
  | "DORMITORY"
  | "PRIVATE"
  | "WORKSHOP"
  | "HIRE"
  | "TRAINING"
  | "MEETING"
  | "ELEVATOR"
  | "CORRIDOR";

export type Rarity = "TIER_1" | "TIER_2" | "TIER_3" | "TIER_4" | "TIER_5" | "TIER_6";

export type Profession =
  | "PIONEER"
  | "WARRIOR"
  | "TANK"
  | "SNIPER"
  | "CASTER"
  | "MEDIC"
  | "SUPPORT"
  | "SPECIAL";

export interface DataStats {
  operators: number;
  operators_by_rarity: Partial<Record<Rarity, number>>;
  skill_tiers: number;
  skill_families: number;
  skill_tiers_by_room: Partial<Record<RoomType, number>>;
  /** Tiers whose description parsed into fully-modelled mechanics. */
  mechanics_parsed: number;
  /** Tiers that parsed but contain explicitly unmodeled parts. */
  mechanics_partial: number;
  /** Tiers the description parser rejected. */
  mechanics_unparsed: number;
  mechanics_coverage_pct: number;
  powers: number;
  facilities: number;
  manufacture_formulas: number;
  layout_slots: number;
}

export interface DataVersion {
  source: string;
  repo: string;
  sha: string;
  locale: string;
  fetched_at: string | null;
  parser_version: string;
}

export interface GameDataVersion extends DataVersion {
  stats: DataStats;
  operators_skipped: number;
}

/** A room kind with its parameters at every level. */
export interface Facility {
  room_type: RoomType;
  name: string;
  description: string;
  category: string;
  max_count: number | null;
  size: { rows: number; cols: number };
  phases: {
    level: number;
    electricity: number;
    max_stationed: number;
    manpower_cost: number;
    build_labor: number;
  }[];
}

/** Something a Factory can make. */
export interface Formula {
  id: string;
  item: string;
  count: number;
  weight: number;
  cost_point: number;
  product: string;
  costs: { item: string; count: number }[];
  require_rooms: { room_type: RoomType; level: number; count: number }[];
}

export interface OperatorSummary {
  id: string;
  name: string;
  rarity: Rarity;
  stars: number;
  profession: Profession;
  sub_profession: string;
  nation: string | null;
  group: string | null;
  team: string | null;
}

// ---- Layer 4: simulation ---------------------------------------------------

/**
 * A simulation request: base, assignment, roster, config and rotation. The
 * full shape is documented by `examples/requests/*.json` in the repository;
 * the server validates it and answers 400 with a reason when it is wrong.
 * `base_id` and `roster_id` may name stored documents in place of `base`
 * and `roster`.
 */
export type SimRequest = Record<string, unknown>;

/** `{ room: [operator id | null, …] }`. */
export type AssignmentMap = Record<string, (string | null)[]>;

/** Tagged unions from the Rust side serialise as `{ kind, ...fields }`. */
export type Tagged = { kind: string } & Record<string, unknown>;

export interface SimTotals {
  lmd: number;
  orundum: number;
  exp: number;
  drones: number;
  lmd_spent: number;
  contacts: number;
  gold_produced: number;
  gold_consumed: number;
  gold_in_depot: number;
  shards_in_depot: number;
  orders_completed: number;
  items: Record<string, number>;
  consumed: Record<string, number>;
}

export interface RoomReport {
  id: string;
  kind: RoomType;
  level: number;
  operators: string[];
  initial_stat_pct: number | null;
  average_stat_pct: number | null;
  produced: Record<string, number>;
  lmd: number;
  orundum: number;
  orders_completed: number;
  drones: number;
  hours_blocked: number;
  in_storage: number;
  pending_orders: number;
  contacts: number;
  training_progress_hours: number;
  training_completed_hour: number | null;
}

export interface OperatorReport {
  id: string;
  name: string;
  initial_mood: number;
  final_mood: number;
  min_mood: number;
  hours_working: number;
  hours_resting: number;
  hours_exhausted: number;
  hours_benched: number;
  hours_idle: number;
}

export interface SimEvent {
  hour: number;
  kind: Tagged;
}

export interface SimResult {
  data: DataVersion;
  horizon_hours: number;
  tick_minutes: number;
  totals: SimTotals;
  rooms: RoomReport[];
  operators: OperatorReport[];
  trajectory: { hour: number; operator: string; mood: number }[];
  events: SimEvent[];
  warnings: Tagged[];
}

export type StatKind =
  | "productivity"
  | "capacity"
  | "order_efficiency"
  | "order_limit"
  | "drone_recovery"
  | "clue_speed"
  | "contact_speed"
  | "training_speed";

export interface RoomStats {
  id: string;
  kind: RoomType;
  level: number;
  headcount: number;
  active: number;
  base_pct: number;
  bonus: Partial<Record<StatKind, number>>;
  productivity_pct: number;
  capacity: number;
  order_efficiency_pct: number;
  order_limit: number;
  drone_recovery_pct: number;
  clue_speed_pct: number;
  contact_speed_pct: number;
  training_speed_pct: number;
}

export interface MoodRate {
  operator: string;
  room: string;
  kind: RoomType;
  base: number;
  skills: number;
  total: number;
  exhausted: boolean;
  idle: boolean;
}

export interface Contribution {
  room: string;
  operator: string;
  skill: string;
  stat: StatKind;
  value: number;
  scaled_from?: number;
}

export interface Snapshot {
  rooms: RoomStats[];
  mood: MoodRate[];
  contributions: Contribution[];
  warnings: Tagged[];
}

// ---- Layer 5: solving --------------------------------------------------------

/**
 * A solve request; see `examples/requests/solve-243.json`. Like a
 * simulation request, it may use `base_id` / `roster_id`.
 */
export type SolveRequest = Record<string, unknown>;

export interface Breakdown {
  lmd: number;
  exp: number;
  orundum: number;
  drones: number;
  contacts: number;
  training_hours: number;
  gold_net: number;
  exhausted_hours: number;
}

export interface Candidate {
  assignment: AssignmentMap;
  inner_score: number;
  score: number;
  breakdown: Breakdown;
  simulated: boolean;
}

export interface SolveResult {
  data: DataVersion;
  strategy: "exhaustive" | "annealing";
  inner: "steady_state" | "simulation";
  space: { variable_slots: number; locked_slots: number; pool: number; estimated_size: number };
  initial: Candidate;
  candidates: Candidate[];
  best_simulation: SimResult | null;
  evaluations: number;
  simulations: number;
  elapsed_ms: number;
  /** Set when the search ended before covering its plan. */
  stopped?: "time_budget" | "requested";
}

/** A running solve's latest report. */
export interface SolveProgress {
  phase: "searching" | "rescoring";
  strategy: "exhaustive" | "annealing";
  /** Units finished in this phase: assignments, annealing steps, or finalists re-scored. */
  done: number;
  total: number;
  evaluations: number;
  initial_score: number;
  best_score: number;
  elapsed_ms: number;
}

// ---- Layer 6: stored documents ---------------------------------------------

export interface DocumentMeta {
  id: string;
  name: string | null;
  schema_version: number;
  created_at: string;
  updated_at: string;
}

export type JobStatus = "pending" | "running" | "done" | "failed" | "cancelled";

/** Where a roster came from: the canonical shape, or another tool's export. */
export type RosterSource = "manual" | "krooster" | "ak-planner";

/** `{ operator id: { promotion: { phase, level }, mood } }`. */
export type Roster = Record<string, { promotion: { phase: "PHASE_0" | "PHASE_1" | "PHASE_2"; level: number }; mood: number | null }>;

/** Something an import skipped or changed; `kind` says what. */
export type ImportWarning =
  | { kind: "unknown_operator"; operator: string }
  | { kind: "promotion_clamped"; operator: string; found: number; used: string; level: number }
  | { kind: "level_clamped"; operator: string; found: number; used: number }
  | { kind: "duplicate"; operator: string }
  | { kind: "no_saved_plan"; operator: string };

/** What reading another tool's export did. */
export interface ImportReport {
  source: "krooster" | "ak-planner";
  format: "krooster_v3" | "krooster_v3_rows" | "krooster_legacy" | "ak_planner_export";
  entries: number;
  imported: number;
  not_owned: number;
  warnings: ImportWarning[];
}

/** A roster as stored (or as a preview would store it). */
export interface RosterView {
  roster: Roster;
  source: RosterSource;
  import?: ImportReport;
}

export interface SolveSummary extends DocumentMeta {
  status: JobStatus;
}

export interface SolveJob extends DocumentMeta {
  status: JobStatus;
  /** The request as it ran: references resolved, server limits applied. */
  request: SolveRequest;
  /** Stored documents the request named. */
  refs?: { base_id?: string; roster_id?: string };
  attempts: number;
  result?: SolveResult;
  error?: string;
  started_at?: string;
  finished_at?: string;
  /** Live, while running. */
  progress?: SolveProgress;
  /** Set once a stop was asked for and the search has not ended yet. */
  stop_requested?: boolean;
}

// ---- transport -------------------------------------------------------------

export class ApiError extends Error {
  constructor(
    public readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

async function parse<T>(res: Response): Promise<T> {
  if (!res.ok) {
    let detail = res.statusText;
    try {
      const body = (await res.json()) as { error?: string };
      if (body.error) detail = body.error;
    } catch {
      // non-JSON error body; keep statusText
    }
    throw new ApiError(res.status, `${res.status} ${detail}`);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

async function getJson<T>(path: string): Promise<T> {
  return parse<T>(await fetch(path, { headers: { Accept: "application/json" } }));
}

async function sendJson<T>(method: "POST" | "PUT", path: string, body: unknown): Promise<T> {
  return parse<T>(
    await fetch(path, {
      method,
      headers: { Accept: "application/json", "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
  );
}

async function deleteJson(path: string): Promise<void> {
  await parse<void>(await fetch(path, { method: "DELETE", headers: { Accept: "application/json" } }));
}

export const api = {
  version: () => getJson<GameDataVersion>("/api/v1/gamedata/version"),
  operators: () => getJson<OperatorSummary[]>("/api/v1/gamedata/operators"),
  facilities: () => getJson<Facility[]>("/api/v1/gamedata/facilities"),
  formulas: () => getJson<Formula[]>("/api/v1/gamedata/formulas"),
  evaluate: (request: SimRequest) => sendJson<Snapshot>("POST", "/api/v1/evaluate", request),
  simulate: (request: SimRequest) => sendJson<SimResult>("POST", "/api/v1/simulate", request),
  solves: {
    create: (request: SolveRequest, name?: string) =>
      sendJson<SolveSummary>("POST", "/api/v1/solves", { name, request }),
    list: () => getJson<SolveSummary[]>("/api/v1/solves"),
    get: (id: string) => getJson<SolveJob>(`/api/v1/solves/${encodeURIComponent(id)}`),
    /** Cancels a pending solve, or ends a running one's search early. */
    stop: (id: string) => sendJson<SolveJob>("POST", `/api/v1/solves/${encodeURIComponent(id)}/stop`, {}),
    remove: (id: string) => deleteJson(`/api/v1/solves/${encodeURIComponent(id)}`),
  },
  rosters: {
    create: (roster: unknown, name?: string, source: RosterSource = "manual") =>
      sendJson<DocumentMeta & { source: RosterSource; import?: ImportReport }>("POST", "/api/v1/rosters", {
        name,
        source,
        roster,
      }),
    /** What an import would store, without storing it. */
    preview: (roster: unknown, source: RosterSource) =>
      sendJson<RosterView>("POST", "/api/v1/rosters/preview", { source, roster }),
    list: () => getJson<DocumentMeta[]>("/api/v1/rosters"),
    get: (id: string) => getJson<DocumentMeta & RosterView>(`/api/v1/rosters/${encodeURIComponent(id)}`),
    remove: (id: string) => deleteJson(`/api/v1/rosters/${encodeURIComponent(id)}`),
  },
  bases: {
    create: (base: unknown, name?: string) => sendJson<DocumentMeta>("POST", "/api/v1/bases", { name, base }),
    list: () => getJson<DocumentMeta[]>("/api/v1/bases"),
    get: (id: string) => getJson<DocumentMeta & { base: unknown }>(`/api/v1/bases/${encodeURIComponent(id)}`),
    remove: (id: string) => deleteJson(`/api/v1/bases/${encodeURIComponent(id)}`),
  },
};
