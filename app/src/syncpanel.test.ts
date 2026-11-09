import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { explanation, peerCompletion, peerState, type Peer } from "./syncpanel";

const peer = (over: Partial<Peer>): Peer => ({ id: "PHONE-1", name: "Phone", connected: false, completion: 100, needItems: 0, lastSeen: null, ...over });

describe("the Sync dialog", () => {
  it("says whether a Sync peer is connected, or when it was last seen", () => {
    const now = new Date(2026, 10, 9, 18, 0).getTime();
    expect(peerState(peer({ connected: true }), now)).toBe("Connected");
    expect(peerState(peer({}), now)).toBe("Never seen");
    expect(peerState(peer({ lastSeen: new Date(2026, 10, 9, 14, 2).toISOString() }), now)).toMatch(/^Last seen today 14.02$/);
    expect(peerState(peer({ lastSeen: new Date(2026, 10, 8, 9, 30).toISOString() }), now)).toMatch(/^Last seen yesterday /);
    expect(peerState(peer({ lastSeen: "garbled" }), now)).toBe("Not connected");
  });

  it("says what a Sync peer still lacks", () => {
    expect(peerCompletion(peer({ completion: 87.9 }))).toBe("has 87%");
    expect(peerCompletion(peer({ completion: 99.99, needItems: 16 }))).toBe("has 99%, lacks 16 files");
    expect(peerCompletion(peer({ completion: 99.99, needItems: 1 }))).toBe("has 99%, lacks 1 file");
    expect(peerCompletion(peer({ completion: 100 }))).toBe("");
  });

  it("explains each way Syncthing can't show a folder", () => {
    expect(explanation({ kind: "notRunning", line: "", address: "http://127.0.0.1:8384" })).toContain("http://127.0.0.1:8384");
    expect(explanation({ kind: "notFound", line: "" })).toContain("works without Syncthing");
    expect(explanation({ kind: "notInFolder", line: "", myId: "ME" })).toContain("isn't in a Syncthing folder");
    expect(explanation({ kind: "folder", line: "", state: "synced", myId: "", folderId: "", folderLabel: "", peers: [], waiting: [] })).toBe("");
  });
});
