// Starting points from `examples/requests/solve-243.json`: a standard 2-4-3
// base (two Trading Posts, four Factories, three Power Plants) and the
// roster that example uses.

import type { BaseConfig, Roster } from "./api";

export const EXAMPLE_BASE: BaseConfig = {
  rooms: [
    { id: "cc", kind: "CONTROL", level: 5 },
    { id: "p1", kind: "POWER", level: 3 },
    { id: "p2", kind: "POWER", level: 3 },
    { id: "p3", kind: "POWER", level: 3 },
    { id: "t1", kind: "TRADING", level: 3, settings: { strategy: "gold" } },
    { id: "t2", kind: "TRADING", level: 3, settings: { strategy: "gold" } },
    { id: "f1", kind: "MANUFACTURE", level: 3, settings: { formula: "4" } },
    { id: "f2", kind: "MANUFACTURE", level: 3, settings: { formula: "4" } },
    { id: "f3", kind: "MANUFACTURE", level: 3, settings: { formula: "3" } },
    { id: "f4", kind: "MANUFACTURE", level: 3, settings: { formula: "3" } },
    { id: "d1", kind: "DORMITORY", level: 5, settings: { ambience: 5000 } },
    { id: "d2", kind: "DORMITORY", level: 5, settings: { ambience: 5000 } },
    { id: "d3", kind: "DORMITORY", level: 5, settings: { ambience: 5000 } },
    { id: "d4", kind: "DORMITORY", level: 5, settings: { ambience: 5000 } },
    { id: "ws", kind: "WORKSHOP", level: 3 },
    { id: "office", kind: "HIRE", level: 3 },
    { id: "training", kind: "TRAINING", level: 3, settings: { training: { profession: "WARRIOR", spec_level: 3 } } },
    { id: "reception", kind: "MEETING", level: 3 },
  ],
};

const at = (phase: "PHASE_0" | "PHASE_1" | "PHASE_2", level: number) => ({ promotion: { phase, level }, mood: null });

export const EXAMPLE_ROSTER: Roster = {
  char_456_ash: at("PHASE_2", 90),
  char_103_angel: at("PHASE_2", 90),
  char_457_blitz: at("PHASE_2", 80),
  char_458_rfrost: at("PHASE_2", 80),
  char_459_tachak: at("PHASE_2", 80),
  char_002_amiya: at("PHASE_2", 80),
  char_102_texas: at("PHASE_2", 80),
  char_140_whitew: at("PHASE_2", 80),
  char_106_franka: at("PHASE_2", 80),
  char_253_greyy: at("PHASE_2", 70),
  char_277_sqrrel: at("PHASE_2", 70),
  char_196_sunbr: at("PHASE_2", 70),
  char_190_clour: at("PHASE_2", 70),
  char_212_ansel: at("PHASE_1", 55),
  char_120_hibisc: at("PHASE_1", 55),
  char_285_medic2: at("PHASE_0", 30),
  char_286_cast3: at("PHASE_0", 30),
  char_124_kroos: at("PHASE_1", 55),
  char_208_melan: at("PHASE_1", 55),
  char_235_jesica: at("PHASE_1", 55),
  char_210_stward: at("PHASE_1", 55),
  char_284_spot: at("PHASE_1", 55),
  char_122_beagle: at("PHASE_1", 55),
  char_123_fang: at("PHASE_1", 55),
  char_240_wyvern: at("PHASE_1", 55),
  char_282_catap: at("PHASE_1", 55),
  char_278_orchid: at("PHASE_1", 55),
  char_150_snakek: at("PHASE_1", 55),
  char_199_yak: at("PHASE_1", 55),
};
