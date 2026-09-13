import {describe, expect, it} from "vitest";

import {jobsPollInterval, updateJob, type JobStatus} from "./jobs";

function job(over: Partial<JobStatus>): JobStatus {
  return {
    name: "example",
    description: "An example job",
    running: false,
    runs: 0,
    last_finished_ms: 0,
    last_started_ms: 0,
    triggerable: true,
    ...over,
  };
}

describe("jobsPollInterval", () => {
  it("polls fast when a job is running", () => {
    expect(jobsPollInterval([job({ running: true })])).toBe(1000);
  });

  it("polls slow when nothing is running", () => {
    expect(jobsPollInterval([job({ running: false }), job({ name: "b", running: false })])).toBe(
      5000,
    );
  });

  it("polls fast if any one of several jobs is running", () => {
    expect(
      jobsPollInterval([job({ running: false }), job({ name: "b", running: true })]),
    ).toBe(1000);
  });

  it("polls slow when there's no data yet", () => {
    expect(jobsPollInterval(undefined)).toBe(5000);
  });

  it("polls slow for an empty job list", () => {
    expect(jobsPollInterval([])).toBe(5000);
  });
});

describe("updateJob", () => {
  it("applies the updater only to the named job, leaving others untouched", () => {
    const jobs = [job({ name: "a" }), job({ name: "b" })];
    const result = updateJob(jobs, "a", (j) => ({ ...j, running: true }));
    expect(result?.find((j) => j.name === "a")?.running).toBe(true);
    expect(result?.find((j) => j.name === "b")?.running).toBe(false);
  });

  it("leaves a different job's own concurrent optimistic update intact", () => {
    // Job "a" was optimistically marked running, then failed; job "b" is a *different* job whose
    // own in-flight optimistic update must survive "a"'s rollback — this is the exact bug found in
    // review: an unscoped rollback clobbering a concurrent, unrelated mutation.
    const jobs = [job({ name: "a", running: true }), job({ name: "b", running: true })];
    const result = updateJob(jobs, "a", (j) => ({ ...j, running: false }));
    expect(result?.find((j) => j.name === "a")?.running).toBe(false);
    expect(result?.find((j) => j.name === "b")?.running).toBe(true);
  });

  it("patches only the field the updater touches, preserving a poll's fresher data on the same job", () => {
    // A poll landing between onMutate and onError may have already refreshed runs/last_ok for this
    // same job (e.g. the trigger actually succeeded server-side even though the client saw an
    // error) — rolling back `running` must not discard that fresher data.
    const jobs = [job({ name: "a", running: true, runs: 6, last_ok: true })];
    const result = updateJob(jobs, "a", (j) => ({ ...j, running: false }));
    const a = result?.find((j) => j.name === "a");
    expect(a?.running).toBe(false);
    expect(a?.runs).toBe(6);
    expect(a?.last_ok).toBe(true);
  });

  it("passes through undefined", () => {
    expect(updateJob(undefined, "a", (j) => j)).toBeUndefined();
  });
});
