// Game data: where the numbers come from, every operator, and the raw
// request panels for developers.

import { useState } from "react";
import type { GameDataVersion, OperatorSummary } from "../api";
import { useGameData } from "../data";
import { Simulator } from "../dev/Simulator";
import { Solver } from "../dev/Solver";
import { professionName, roomName } from "../format";

function VersionCard({ v }: { v: GameDataVersion }) {
  const commitUrl = `https://github.com/${v.repo}/commit/${v.sha}`;
  const rooms = Object.entries(v.stats.skill_tiers_by_room).sort(([, a], [, b]) => (b ?? 0) - (a ?? 0));
  return (
    <div className="card">
      <h2 className="card-title">Source</h2>
      <dl>
        <dt>Data</dt>
        <dd>
          {v.source} · <a href={commitUrl}>{v.sha.slice(0, 12)}</a> ({v.repo})
        </dd>
        <dt>Fetched</dt>
        <dd>{v.fetched_at ?? "unknown"}</dd>
        <dt>Parser</dt>
        <dd>ak-data {v.parser_version}</dd>
        <dt>Operators</dt>
        <dd>
          {v.stats.operators} loaded
          {v.operators_skipped > 0 && <span className="warn"> · {v.operators_skipped} skipped</span>}
        </dd>
        <dt>Base skills</dt>
        <dd>
          {v.stats.skill_tiers} tiers in {v.stats.skill_families} families; {v.stats.mechanics_parsed} fully modelled (
          {v.stats.mechanics_coverage_pct.toFixed(1)}%)
          {v.stats.mechanics_partial > 0 && <span className="muted"> · {v.stats.mechanics_partial} partial</span>}
          {v.stats.mechanics_unparsed > 0 && <span className="warn"> · {v.stats.mechanics_unparsed} rejected</span>}
        </dd>
      </dl>
      <table className="compact">
        <thead>
          <tr>
            <th>Room</th>
            <th className="num">Skill tiers</th>
          </tr>
        </thead>
        <tbody>
          {rooms.map(([room, n]) => (
            <tr key={room}>
              <td>{roomName(room as never)}</td>
              <td className="num">{n}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function OperatorTable({ ops }: { ops: OperatorSummary[] }) {
  const [query, setQuery] = useState("");
  const q = query.trim().toLowerCase();
  const shown = ops
    .filter((o) => !q || o.name.toLowerCase().includes(q) || o.id.includes(q))
    .sort((a, b) => b.stars - a.stars || a.name.localeCompare(b.name));
  return (
    <div className="card">
      <h2 className="card-title">Operators</h2>
      <input
        type="search"
        placeholder="Filter by name or id"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        aria-label="Filter operators"
      />
      <p className="muted small">
        {shown.length} of {ops.length}
      </p>
      <div className="table-scroll">
        <table className="compact-rows">
          <thead>
            <tr>
              <th>Name</th>
              <th>★</th>
              <th>Class</th>
              <th>Subclass</th>
              <th>Faction</th>
              <th className="num">Max level</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((o) => (
              <tr key={o.id}>
                <td title={o.id}>{o.name}</td>
                <td>{o.stars}</td>
                <td>{professionName(o.profession)}</td>
                <td>{o.sub_profession}</td>
                <td>{[o.nation, o.group, o.team].filter(Boolean).join(" · ")}</td>
                <td className="num">
                  E{o.max_levels.length - 1} L{o.max_levels.at(-1)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

export function DataView() {
  const data = useGameData();
  const ops = [...data.operators.values()];
  return (
    <div className="stack">
      <VersionCard v={data.version} />
      <OperatorTable ops={ops} />
      <details className="card">
        <summary>Developer tools: raw simulate and solve requests</summary>
        <h3>Simulator</h3>
        <Simulator names={new Map(ops.map((o) => [o.id, o.name]))} />
        <h3>Solver</h3>
        <Solver names={new Map(ops.map((o) => [o.id, o.name]))} />
      </details>
    </div>
  );
}
