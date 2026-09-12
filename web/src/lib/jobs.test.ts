import {describe, expect, it} from "vitest";

import {jobsPollInterval, type JobStatus} from "./jobs";

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
