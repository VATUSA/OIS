import {describe, expect, it} from "vitest";

import {jobsPollInterval, withJobRestored, withJobRunning, type JobStatus} from "./jobs";

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

describe("withJobRunning", () => {
  it("marks only the named job as running", () => {
    const jobs = [job({ name: "a" }), job({ name: "b" })];
    const result = withJobRunning(jobs, "a");
    expect(result?.find((j) => j.name === "a")?.running).toBe(true);
    expect(result?.find((j) => j.name === "b")?.running).toBe(false);
  });

  it("passes through undefined", () => {
    expect(withJobRunning(undefined, "a")).toBeUndefined();
  });
});

describe("withJobRestored", () => {
  it("restores only the named job, leaving a different job's own concurrent update intact", () => {
    // Job "a" was optimistically marked running, then failed; job "b" is a *different* job whose
    // own in-flight optimistic update must survive "a"'s rollback — this is the exact bug found in
    // review: an unscoped rollback snapshot clobbering a concurrent, unrelated mutation.
    const jobs = [job({ name: "a", running: true }), job({ name: "b", running: true })];
    const result = withJobRestored(jobs, "a", job({ name: "a", running: false }));
    expect(result?.find((j) => j.name === "a")?.running).toBe(false);
    expect(result?.find((j) => j.name === "b")?.running).toBe(true);
  });

  it("passes through undefined", () => {
    expect(withJobRestored(undefined, "a", job({ name: "a" }))).toBeUndefined();
  });
});
