// construct: import-default
import describe from "./describe";
// construct: import-namespace
import * as units from "./units";
// construct: import-path-alias, import-inline-type
import { Circle, type Shape } from "@/models";
// construct: import-side-effect
import "./setup";

export class ShapeService {
  private shapes: Shape[] = [];

  // construct: method-returning-this
  add(shape: Shape): this {
    this.shapes.push(shape);
    return this;
  }

  total(): number {
    // construct: arrow-callback
    return this.shapes.reduce((sum, shape) => sum + shape.area(), 0);
  }

  // construct: async-method
  async report(): Promise<string> {
    const [unit] = await units.load();
    return this.shapes.map((shape) => `${describe(shape)} ${unit}`).join("\n");
  }
}

// construct: top-level-statement
export const service = new ShapeService().add(new Circle(1));
