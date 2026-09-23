// The base drawn on the game's own floor plan (`building_data.layouts.v0`):
// every room in a slot of its category and size, with who is stationed
// there. With a `baseline`, operators that differ from it are marked.

import type { AssignmentMap, BaseConfig, Facility, LayoutSlot, Room, RoomType } from "../api";
import { useGameData } from "../data";
import { itemName, roomName } from "../format";

/** Slots that never hold a staffable room. */
const HIDDEN = new Set(["ELEVATOR", "CORRIDOR", "CUSTOM_P"]);

/** A colour wash per slot category; the room's kind is always written out. */
const TONE: Record<string, string> = { OUTPUT: "tone-1", CUSTOM: "tone-2", SPECIAL: "tone-3", FUNCTION: "tone-3" };

const CW = 34; // px per grid column
const CH = 36; // px per grid row
const GUTTER = 1.4; // columns reserved for floor labels

/**
 * Assigns each room the first free slot of its category and size, upper
 * floors first (B1 before B2), left to right. The game lets players choose;
 * this only has to be stable and plausible.
 */
export function placeRooms(
  base: BaseConfig,
  slots: LayoutSlot[],
  facilities: Map<RoomType, Facility>,
): { placed: Map<string, LayoutSlot>; unplaced: Room[] } {
  const free = slots
    .filter((s) => !HIDDEN.has(s.category))
    .sort((a, b) => b.offset.row - a.offset.row || a.offset.col - b.offset.col);
  const used = new Set<string>();
  const placed = new Map<string, LayoutSlot>();
  const unplaced: Room[] = [];
  for (const room of base.rooms) {
    const f = facilities.get(room.kind);
    const slot = free.find(
      (s) =>
        !used.has(s.id) &&
        f !== undefined &&
        s.category === f.category &&
        s.size.rows === f.size.rows &&
        s.size.cols === f.size.cols,
    );
    if (slot) {
      used.add(slot.id);
      placed.set(room.id, slot);
    } else {
      unplaced.push(room);
    }
  }
  return { placed, unplaced };
}

/** Occupants that differ between two assignments, per room. */
export function diffRoom(room: string, now?: AssignmentMap, before?: AssignmentMap) {
  const a = new Set((now?.[room] ?? []).filter((x): x is string => x !== null));
  const b = new Set((before?.[room] ?? []).filter((x): x is string => x !== null));
  return {
    added: [...a].filter((x) => !b.has(x)),
    removed: [...b].filter((x) => !a.has(x)),
  };
}

function clip(text: string, px: number, charPx = 6.2): string {
  const max = Math.max(3, Math.floor(px / charPx));
  return text.length <= max ? text : `${text.slice(0, max - 1)}…`;
}

function detail(room: Room, formulas: Map<string, { item: string }>): string {
  const s = room.settings ?? {};
  switch (room.kind) {
    case "MANUFACTURE": {
      const f = s.formula ? formulas.get(s.formula) : undefined;
      return f ? itemName(f.item).replace(" Battle Record", " Record") : "idle";
    }
    case "TRADING":
      return s.strategy === "originium_shard" ? "Orundum" : "LMD";
    case "DORMITORY":
      return `ambience ${s.ambience ?? 0}`;
    case "TRAINING":
      return s.training ? `S${s.training.spec_level}` : "idle";
    default:
      return "";
  }
}

interface Props {
  base: BaseConfig;
  /** Who is where; omit to draw the rooms only. */
  assignment?: AssignmentMap;
  /** Mark operators that differ from this assignment. */
  baseline?: AssignmentMap;
  /** Accessible name for the drawing. */
  label: string;
}

export function BaseMap({ base, assignment, baseline, label }: Props) {
  const { layout, facilities, formulas, name } = useGameData();
  const visible = layout.slots.filter((s) => !HIDDEN.has(s.category));
  const { placed, unplaced } = placeRooms(base, layout.slots, facilities);
  const maxRow = Math.max(...visible.map((s) => s.offset.row + s.size.rows));
  const maxCol = Math.max(...visible.map((s) => s.offset.col + s.size.cols));
  const width = (maxCol + GUTTER) * CW;
  const height = maxRow * CH;
  const x = (col: number) => (col + GUTTER) * CW;
  const y = (row: number, rows: number) => (maxRow - row - rows) * CH;
  const floors = new Map<string, number>();
  for (const s of visible) if (s.storey && !floors.has(s.storey)) floors.set(s.storey, s.offset.row);
  const occupied = new Set(placed.values());

  return (
    <div className="map-scroll">
      <svg className="base-map" viewBox={`0 0 ${width} ${height}`} role="img" aria-label={label}>
        {[...floors].map(([storey, row]) => (
          <text key={storey} className="floor" x={4} y={y(row, 2) + CH + 4}>
            {storey}
          </text>
        ))}
        {visible
          .filter((s) => !occupied.has(s))
          .map((s) => (
            <rect
              key={s.id}
              className="slot-empty"
              x={x(s.offset.col) + 2}
              y={y(s.offset.row, s.size.rows) + 2}
              width={s.size.cols * CW - 4}
              height={s.size.rows * CH - 4}
              rx={6}
            />
          ))}
        {base.rooms.map((room) => {
          const slot = placed.get(room.id);
          const f = facilities.get(room.kind);
          if (!slot || !f) return null;
          const rx = x(slot.offset.col) + 2;
          const ry = y(slot.offset.row, slot.size.rows) + 2;
          const w = slot.size.cols * CW - 4;
          const h = slot.size.rows * CH - 4;
          const cap = f.phases[room.level - 1]?.max_stationed ?? 0;
          const ops = assignment?.[room.id] ?? [];
          const filled = ops.filter((o) => o !== null).length;
          const { added, removed } = baseline
            ? diffRoom(room.id, assignment, baseline)
            : { added: [] as string[], removed: [] as string[] };
          const changed = added.length > 0 || removed.length > 0;
          const extra = detail(room, formulas);
          const title = `${roomName(room.kind)} L${room.level}`;
          // Names in one column, or two for wide rooms with many slots.
          const columns = cap > 3 && slot.size.rows <= 2 ? 2 : 1;
          const colWidth = (w - 12) / columns;
          const perColumn = Math.ceil(Math.max(cap, 1) / columns);
          const lines = ops
            .map((op, i) => ({ op, i }))
            .filter((l): l is { op: string; i: number } => l.op !== null);
          return (
            <g key={room.id} className={`room ${TONE[f.category] ?? "tone-3"}${changed ? " changed" : ""}`}>
              <title>
                {[
                  `${title}${extra ? ` · ${extra}` : ""} (${room.id})`,
                  ...lines.map((l) => name(l.op) + (added.includes(l.op) ? " (new)" : "")),
                  ...(removed.length ? [`moved out: ${removed.map(name).join(", ")}`] : []),
                ].join("\n")}
              </title>
              <rect x={rx} y={ry} width={w} height={h} rx={6} />
              <text className="room-title" x={rx + 6} y={ry + 13}>
                {clip(title, w - (assignment ? 36 : 12), 6.1)}
              </text>
              {extra && (
                <text className="room-detail" x={rx + 6} y={ry + 25}>
                  {clip(extra, w - 12, 5.4)}
                </text>
              )}
              {assignment && (
                <text className="room-count" x={rx + w - 6} y={ry + 13} textAnchor="end">
                  {filled}/{cap}
                </text>
              )}
              {lines.map(({ op, i }, n) => {
                const column = Math.floor(n / perColumn);
                const row = n % perColumn;
                const isNew = added.includes(op);
                const trainee = room.kind === "TRAINING" && i === 1;
                const text = `${isNew ? "+ " : ""}${name(op)}${trainee ? " (trainee)" : ""}`;
                return (
                  <text
                    key={op}
                    className={isNew ? "op op-new" : "op"}
                    x={rx + 6 + column * colWidth}
                    y={ry + (extra ? 38 : 29) + row * 12}
                  >
                    {clip(text, colWidth - 4)}
                  </text>
                );
              })}
            </g>
          );
        })}
      </svg>
      {unplaced.length > 0 && (
        <p className="warn">
          No slot left for: {unplaced.map((r) => `${roomName(r.kind)} (${r.id})`).join(", ")}
        </p>
      )}
    </div>
  );
}
