import { describe, it, expect } from "vitest";

describe("standalone", () => {
  it("does not import from src", () => {
    expect(true).toBe(true);
  });
});
