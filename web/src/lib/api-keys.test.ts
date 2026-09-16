import {describe, expect, it} from "vitest";

import {type ApiKeyToken, withReveal, withoutReveal} from "./api-keys";

const token = (keyId: string, secret: string) => ({ key: { id: keyId }, token: secret }) as ApiKeyToken;

describe("token reveals", () => {
  // Each plaintext token is shown exactly once, so a new reveal must never push out another key's.
  it("keeps every key's unrevealed token when another key is created or rotated", () => {
    const list = withReveal(withReveal([], token("a", "ois_pat_a1")), token("b", "ois_pat_b1"));
    expect(list.map((t) => t.token)).toEqual(["ois_pat_b1", "ois_pat_a1"]);
  });

  it("replaces a key's earlier token when that same key is rotated again (the old one is dead)", () => {
    const list = withReveal(withReveal([token("a", "ois_pat_a1"), token("b", "ois_pat_b1")], token("a", "ois_pat_a2")), token("c", "ois_pat_c1"));
    expect(list.map((t) => t.token)).toEqual(["ois_pat_c1", "ois_pat_a2", "ois_pat_b1"]);
  });

  it("dismisses only the chosen key's token", () => {
    expect(withoutReveal([token("a", "1"), token("b", "2")], "a").map((t) => t.key.id)).toEqual(["b"]);
  });
});
