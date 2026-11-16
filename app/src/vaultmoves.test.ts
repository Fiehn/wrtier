import { describe, expect, it } from "vitest";
import { destinations, movedFolders, whereTo, type Taken } from "./vaultmoves";

const taken: Taken = {
  works: ["Rain", "Short stories/Storm", "Short stories/Old/Fog"],
  folders: ["Short stories", "Short stories/Old", "Old", "Notes"],
};
const work = (id: string) => ({ kind: "work" as const, id });
const folder = (path: string) => ({ kind: "folder" as const, path });

describe("where a move goes", () => {
  it("puts a Work into a folder, or at the top of the Vault", () => {
    expect(whereTo(work("Rain"), "Short stories", taken)).toEqual({ to: "Short stories/Rain" });
    expect(whereTo(work("Short stories/Storm"), "", taken)).toEqual({ to: "Storm" });
    expect(whereTo(work("Short stories/Old/Fog"), "Notes", taken)).toEqual({ to: "Notes/Fog" });
  });

  it("moves a folder with its name, into another or to the top", () => {
    expect(whereTo(folder("Short stories/Old"), "Notes", taken)).toEqual({ to: "Notes/Old" });
    expect(whereTo(folder("Notes"), "Short stories/Old", taken)).toEqual({ to: "Short stories/Old/Notes" });
  });

  it("does nothing where it already is", () => {
    expect(whereTo(work("Short stories/Storm"), "Short stories", taken)).toEqual({ same: true });
    expect(whereTo(work("Rain"), "", taken)).toEqual({ same: true });
    expect(whereTo(folder("Short stories/Old"), "Short stories", taken)).toEqual({ same: true });
  });

  it("refuses a folder into itself, and a name that's taken (ignoring case)", () => {
    expect(whereTo(folder("Short stories"), "Short stories", taken)).toEqual({ refused: "A folder can't go inside itself." });
    expect(whereTo(folder("Short stories"), "Short stories/Old", taken)).toEqual({ refused: "A folder can't go inside itself." });
    expect(whereTo(folder("Short stories/Old"), "", taken)).toEqual({ refused: "There's already a folder called Old." });
    expect(whereTo(work("Short stories/Old/Fog"), "", { ...taken, works: [...taken.works, "fog"] })).toEqual({ refused: "There's already a Work called Fog." });
    // A folder that only starts the same isn't inside it.
    expect(whereTo(folder("Short"), "Short stories", { works: [], folders: ["Short", "Short stories"] })).toEqual({ to: "Short stories/Short" });
  });
});

describe("Move to…", () => {
  const folders = ["Notes", "Short stories", "Short stories/Old"];
  it("offers the top and every folder but where it is", () => {
    expect(destinations(work("Short stories/Storm"), folders)).toEqual(["", "Notes", "Short stories/Old"]);
    expect(destinations(work("Rain"), folders)).toEqual(["Notes", "Short stories", "Short stories/Old"]);
  });

  it("never offers a folder itself or what's inside it", () => {
    expect(destinations(folder("Short stories"), folders)).toEqual(["Notes"]);
    expect(destinations(folder("Short stories/Old"), folders)).toEqual(["", "Notes"]);
  });

  it("keeps open folders open under their new names", () => {
    expect(movedFolders(["Short stories", "Short stories/Old", "Short", "Notes"], "Short stories", "Notes/Short stories"))
      .toEqual(["Notes/Short stories", "Notes/Short stories/Old", "Short", "Notes"]);
  });
});
