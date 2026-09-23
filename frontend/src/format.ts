// Display helpers: numbers, names of things the data only has ids for, and
// error text.

import type { Formula, Profession, Roster, RoomType } from "./api";

export const fmt = (n: number, digits = 0) =>
  n.toLocaleString(undefined, { minimumFractionDigits: digits, maximumFractionDigits: digits });

/** 1,284 / 12.9K / 4.2M, for tiles. */
export function compact(n: number): string {
  const abs = Math.abs(n);
  if (abs >= 1e6) return `${fmt(n / 1e6, 1)}M`;
  if (abs >= 1e4) return `${fmt(n / 1e3, 1)}K`;
  return fmt(n, abs < 10 && n !== Math.round(n) ? 1 : 0);
}

/** A total over `hours`, as a per-day rate. */
export const perDay = (total: number, hours: number) => (hours > 0 ? (total * 24) / hours : 0);

/** Signed percentage change, "+3.4%". */
export function change(now: number, before: number): string {
  if (before === 0) return now === 0 ? "±0%" : "new";
  const pct = ((now - before) / Math.abs(before)) * 100;
  const sign = pct > 0 ? "+" : pct < 0 ? "−" : "±";
  return `${sign}${fmt(Math.abs(pct), 1)}%`;
}

// The pinned data has no item table, so the items Factory formulas make or
// use are named here for display. Ids are upstream item ids.
const ITEM_NAMES: Record<string, string> = {
  "2001": "Drill Battle Record",
  "2002": "Frontline Battle Record",
  "2003": "Tactical Battle Record",
  "2004": "Strategic Battle Record",
  "3003": "Pure Gold",
  "3141": "Originium Shard",
  "4001": "LMD",
  "30012": "Orirock Cube",
  "30062": "Device",
  "32001": "Chip Catalyst",
  "3212": "Vanguard Chip Pack",
  "3213": "Vanguard Dualchip",
  "3222": "Guard Chip Pack",
  "3223": "Guard Dualchip",
  "3232": "Defender Chip Pack",
  "3233": "Defender Dualchip",
  "3242": "Sniper Chip Pack",
  "3243": "Sniper Dualchip",
  "3252": "Caster Chip Pack",
  "3253": "Caster Dualchip",
  "3262": "Medic Chip Pack",
  "3263": "Medic Dualchip",
  "3272": "Supporter Chip Pack",
  "3273": "Supporter Dualchip",
  "3282": "Specialist Chip Pack",
  "3283": "Specialist Dualchip",
};

export const itemName = (id: string) => ITEM_NAMES[id] ?? `item ${id}`;

/**
 * A formula's name: what it makes, what it is made from when another
 * formula makes the same thing, and the Factory level it needs.
 */
export function formulaLabel(f: Formula, all: Iterable<Formula> = []): string {
  const level = f.require_rooms.find((r) => r.room_type === "MANUFACTURE")?.level ?? 1;
  const twin = [...all].some((g) => g.id !== f.id && g.item === f.item);
  const input = f.costs.find((c) => c.item !== "4001");
  const from = twin && input ? ` from ${itemName(input.item)}` : "";
  return `${itemName(f.item)}${from}${level > 1 ? ` (level ${level})` : ""}`;
}

/** Formulas in upstream order: Battle Records, Pure Gold, Dualchips, Shards. */
export const byFormulaId = (a: Formula, b: Formula) => Number(a.id) - Number(b.id);

export const PROFESSIONS: { id: Profession; name: string }[] = [
  { id: "PIONEER", name: "Vanguard" },
  { id: "WARRIOR", name: "Guard" },
  { id: "TANK", name: "Defender" },
  { id: "SNIPER", name: "Sniper" },
  { id: "CASTER", name: "Caster" },
  { id: "MEDIC", name: "Medic" },
  { id: "SUPPORT", name: "Supporter" },
  { id: "SPECIAL", name: "Specialist" },
];

export const professionName = (p: Profession) => PROFESSIONS.find((x) => x.id === p)?.name ?? p;

/** Short room-kind names, where the game's own are long. */
export const ROOM_NAMES: Partial<Record<RoomType, string>> = {
  CONTROL: "Control Center",
  POWER: "Power Plant",
  MANUFACTURE: "Factory",
  TRADING: "Trading Post",
  DORMITORY: "Dormitory",
  WORKSHOP: "Workshop",
  HIRE: "Office",
  TRAINING: "Training Room",
  MEETING: "Reception Room",
};

export const roomName = (kind: RoomType) => ROOM_NAMES[kind] ?? kind;

export function promotion(entry: Roster[string]): string {
  const elite = entry.promotion.phase.replace("PHASE_", "E");
  return `${elite} L${entry.promotion.level}`;
}

export const errorText = (err: unknown) => (err instanceof Error ? err.message : String(err));

export const when = (iso: string) => new Date(iso).toLocaleString();
