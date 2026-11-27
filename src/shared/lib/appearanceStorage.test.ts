import { expect, it } from "vitest";
import { mirrorAppearance, mirroredAppearance } from "./appearance";

it("still follows System when browser storage is unavailable", () => {
  expect(() => mirrorAppearance("dark")).not.toThrow();
  expect(mirroredAppearance()).toBe("system");
});
