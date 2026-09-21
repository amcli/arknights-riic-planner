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

export interface GameDataVersion {
  source: string;
  repo: string;
  sha: string;
  locale: string;
  fetched_at: string | null;
  parser_version: string;
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

export class ApiError extends Error {
  constructor(
    public readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

async function getJson<T>(path: string): Promise<T> {
  const res = await fetch(path, { headers: { Accept: "application/json" } });
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

export const api = {
  version: () => getJson<GameDataVersion>("/api/v1/gamedata/version"),
  operators: () => getJson<OperatorSummary[]>("/api/v1/gamedata/operators"),
};
