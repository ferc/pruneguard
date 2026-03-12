import { describe, it, expect } from "vitest";
import { add, subtract } from "../math";

describe("math", () => {
  it("adds numbers", () => {
    expect(add(1, 2)).toBe(3);
  });

  it("subtracts numbers", () => {
    expect(subtract(3, 1)).toBe(2);
  });
});
