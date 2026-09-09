import { describe, expect, it } from "vitest";
import type { CollabBoardCard, CollabBoardColumn } from "../api";
import { agentFlowGroups } from "./AgentFlow";

const card = (id: string, owner: string, status = "in-progress"): CollabBoardCard =>
  ({ id, title: id, owner, status, work: `work ${id}` }) as unknown as CollabBoardCard;

const column = (name: string, cards: CollabBoardCard[]): CollabBoardColumn =>
  ({ name, cards }) as CollabBoardColumn;

describe("agentFlowGroups", () => {
  it("groups cards by owner and keeps the lane", () => {
    const groups = agentFlowGroups([
      column("needs-review", [card("a", "agent-a"), card("b", "agent-b")]),
      column("in-progress", [card("c", "agent-a")]),
    ]);
    expect(groups.map((g) => g.owner)).toEqual(["agent-a", "agent-b"]);
    expect(groups[0]?.cards.map((item) => `${item.card.id}:${item.lane}`)).toEqual([
      "a:needs-review",
      "c:in-progress",
    ]);
  });

  it("uses one unassigned bucket for empty owners", () => {
    const groups = agentFlowGroups([column("open", [card("a", ""), card("b", "  ")])]);
    expect(groups).toHaveLength(1);
    expect(groups[0]?.owner).toBe("unassigned");
    expect(groups[0]?.cards.map((item) => item.card.id)).toEqual(["a", "b"]);
  });

  it("omits archived cards", () => {
    const groups = agentFlowGroups([
      column("closed", [card("old", "agent-a", "closed")]),
      column("in-progress", [card("active", "agent-a")]),
    ]);
    expect(groups[0]?.cards.map((item) => item.card.id)).toEqual(["active"]);
  });
});
