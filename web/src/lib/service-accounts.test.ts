import {describe, expect, it} from "vitest";

import {type ServiceAccountToken, withReveal, withoutReveal} from "./service-accounts";

const token = (accountId: string, secret: string) =>
  ({account: {id: accountId}, token: secret}) as ServiceAccountToken;

describe("token reveals", () => {
  // Each plaintext token is shown exactly once, so a new reveal must never push out another
  // account's — losing it means the account has to be rotated before it can be used at all.
  it("keeps every account's unrevealed token when another is created", () => {
    const list = withReveal(withReveal([], token("a", "ois_sa_a1")), token("b", "ois_sa_b1"));
    expect(list.map((t) => t.token)).toEqual(["ois_sa_b1", "ois_sa_a1"]);
  });

  it("replaces an account's earlier token when it is re-issued (the old one is dead)", () => {
    const list = withReveal(
      withReveal([token("a", "ois_sa_a1"), token("b", "ois_sa_b1")], token("a", "ois_sa_a2")),
      token("c", "ois_sa_c1"),
    );
    expect(list.map((t) => t.token)).toEqual(["ois_sa_c1", "ois_sa_a2", "ois_sa_b1"]);
  });

  it("dismisses only the chosen account's token", () => {
    expect(withoutReveal([token("a", "1"), token("b", "2")], "a").map((t) => t.account.id)).toEqual([
      "b",
    ]);
  });
});
