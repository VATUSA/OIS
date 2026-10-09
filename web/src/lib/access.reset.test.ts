import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";

const get = vi.hoisted(() => vi.fn());

// The generated client captures `fetch` when its module loads, so mock the client itself (#387).
vi.mock("./api", () => ({ois: {GET: get}}));

import {waitForReset} from "./access";

const running = {id: "run-1", status: "running", started_at: "2026-10-09T12:00:00Z", finished_at: null, result: null, failure: null};
const result = {dry_run: false, pull_summary: "ok", users_checked: 40, users_reset: 2, users: []};

/** Answer the polls in turn: `undefined` is a failed poll, `"throw"` a network error. */
function answer(...polls: (object | undefined | "throw")[]) {
  for (const poll of polls) {
    if (poll === "throw") get.mockRejectedValueOnce(new Error("network"));
    else get.mockResolvedValueOnce({data: poll, error: poll ? undefined : {status: 502}});
  }
}

/** Run `waitForReset` to its end under fake timers; resolves to what it returned or threw. */
async function settle(id = "run-1") {
  const outcome = waitForReset(id).then(
    (value) => ({value}),
    (error: Error & {usersReset?: number}) => ({error}),
  );
  await vi.runAllTimersAsync();
  return outcome;
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  vi.useRealTimers();
  get.mockReset();
});

describe("waiting for an access reset run (VATUSA/OIS#806)", () => {
  it("polls the run until it finishes and returns its result", async () => {
    answer(running, running, {...running, status: "succeeded", result});
    expect(await settle()).toEqual({value: result});
    expect(get).toHaveBeenCalledTimes(3);
    expect(get).toHaveBeenCalledWith("/api/v1/admin/access/vatusa-reset/runs/{id}", {params: {path: {id: "run-1"}}});
  });

  it("throws a failed run's message with how many users it reset", async () => {
    answer({...running, status: "failed", failure: {error: "reset_interrupted", message: "stopped", users_reset: 7}});
    const {error} = (await settle()) as {error: Error & {usersReset?: number}};
    expect(error.message).toBe("stopped");
    expect(error.usersReset).toBe(7);
  });

  it("gives up after five failed polls in a row, and says the run carries on", async () => {
    answer(undefined, "throw", undefined, undefined, undefined, running);
    const {error} = (await settle()) as {error: Error};
    expect(error.message).toContain("lost track of the reset; it carries on on the server");
    expect(get).toHaveBeenCalledTimes(5);
  });

  it("starts counting failed polls again after an answer", async () => {
    answer(undefined, undefined, undefined, undefined, running, undefined, undefined, undefined, undefined, {
      ...running,
      status: "succeeded",
      result,
    });
    expect(await settle()).toEqual({value: result});
    expect(get).toHaveBeenCalledTimes(10);
  });

  it("does not report a finished run whose result is missing as a success", async () => {
    answer({...running, status: "succeeded", result: null});
    const {error} = (await settle()) as {error: Error};
    expect(error.message).toBe("the reset finished, but its result could not be read");
  });
});
