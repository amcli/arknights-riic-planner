// Typed client for the ak-api endpoints that exist today. Types mirror the
// serde output of the Rust domain structs; keep them in sync by hand until
// an OpenAPI/TS generator is wired in.

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
 */
export type SimRequest = Record<string, unknown>;

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
  return (await res.json()) as T;
}

async function getJson<T>(path: string): Promise<T> {
  return parse<T>(await fetch(path, { headers: { Accept: "application/json" } }));
}

async function postJson<T>(path: string, body: unknown): Promise<T> {
  return parse<T>(
    await fetch(path, {
      method: "POST",
      headers: { Accept: "application/json", "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }),
  );
}

export const api = {
  version: () => getJson<GameDataVersion>("/api/v1/gamedata/version"),
  operators: () => getJson<OperatorSummary[]>("/api/v1/gamedata/operators"),
  evaluate: (request: SimRequest) => postJson<Snapshot>("/api/v1/evaluate", request),
  simulate: (request: SimRequest) => postJson<SimResult>("/api/v1/simulate", request),
};
