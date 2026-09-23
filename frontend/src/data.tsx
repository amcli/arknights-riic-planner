// The game data every screen needs, fetched once and shared through
// context.

import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import {
  api,
  type BaseLayout,
  type Facility,
  type Formula,
  type GameConstants,
  type GameDataVersion,
  type OperatorSummary,
  type RoomType,
} from "./api";
import { errorText } from "./format";

export interface GameData {
  version: GameDataVersion;
  operators: Map<string, OperatorSummary>;
  facilities: Map<RoomType, Facility>;
  formulas: Map<string, Formula>;
  layout: BaseLayout;
  constants: GameConstants;
  /** An operator's name, or the id if the data lacks it. */
  name: (id: string) => string;
}

const Context = createContext<GameData | null>(null);

async function load(): Promise<GameData> {
  const [version, operators, facilities, formulas, layout, constants] = await Promise.all([
    api.version(),
    api.operators(),
    api.facilities(),
    api.formulas(),
    api.layout(),
    api.constants(),
  ]);
  const byId = new Map(operators.map((o) => [o.id, o]));
  return {
    version,
    operators: byId,
    facilities: new Map(facilities.map((f) => [f.room_type, f])),
    formulas: new Map(formulas.map((f) => [f.id, f])),
    layout,
    constants,
    name: (id) => byId.get(id)?.name ?? id,
  };
}

export function GameDataProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<{ data?: GameData; error?: string }>({});
  useEffect(() => {
    let cancelled = false;
    load().then(
      (data) => !cancelled && setState({ data }),
      (err: unknown) => !cancelled && setState({ error: errorText(err) }),
    );
    return () => {
      cancelled = true;
    };
  }, []);
  if (state.error) {
    return (
      <div className="card">
        <p className="error">Could not reach the API ({state.error}).</p>
        <p className="muted">
          Start it with <code>cargo run -p ak-api</code>, then reload this page.
        </p>
      </div>
    );
  }
  if (!state.data) return <p className="muted">Loading game data…</p>;
  return <Context.Provider value={state.data}>{children}</Context.Provider>;
}

export function useGameData(): GameData {
  const data = useContext(Context);
  if (!data) throw new Error("useGameData outside GameDataProvider");
  return data;
}
