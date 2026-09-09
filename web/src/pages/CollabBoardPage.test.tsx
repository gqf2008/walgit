import { describe, expect, it } from "vitest";
import type { CollabBoardColumn } from "../api";
import { visibleBoardColumns } from "./CollabBoardPage";

const column = (name: string, cards: number): CollabBoardColumn =>
  ({ name, cards: Array.from({ length: cards }, () => ({})) }) as unknown as CollabBoardColumn;

describe("visibleBoardColumns", () => {
  it("hides empty lanes by default", () => {
    const columns = [column("open", 1), column("in-progress", 0), column("closed", 2)];
    expect(visibleBoardColumns(columns, false).map((c) => c.name)).toEqual(["open", "closed"]);
  });

  it("shows every lane when asked", () => {
    const columns = [column("open", 1), column("in-progress", 0), column("closed", 2)];
    expect(visibleBoardColumns(columns, true).map((c) => c.name)).toEqual(["open", "in-progress", "closed"]);
  });

  it("keeps an all-empty board collapsed by default", () => {
    const columns = [column("open", 0), column("in-progress", 0)];
    expect(visibleBoardColumns(columns, false)).toEqual([]);
    expect(visibleBoardColumns(columns, true).map((c) => c.name)).toEqual(["open", "in-progress"]);
  });
});
