import type { Shape } from "./models";

// construct: export-default-anonymous
export default function (shape: Shape): string {
  return shape.label ?? shape.kind;
}
